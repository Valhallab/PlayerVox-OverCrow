use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::{
    integration::{is_normal_absolute_path, run_bounded_command, run_bounded_command_status},
    update_download::VerifiedPackage,
    updates::{PackageTarget, UpdateError},
};

const CONTROL_BINARY: &str = "/usr/bin/overcrow-control";
const PKEXEC: &str = "/usr/bin/pkexec";
const PACMAN: &str = "/usr/bin/pacman";
const RPM: &str = "/usr/bin/rpm";
const DPKG_QUERY: &str = "/usr/bin/dpkg-query";
const DNF5: &str = "/usr/bin/dnf5";
const DNF: &str = "/usr/bin/dnf";
const APT_GET: &str = "/usr/bin/apt-get";
const RPM_OSTREE: &str = "/usr/bin/rpm-ostree";
const OSTREE_BOOTED: &str = "/run/ostree-booted";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_OS_RELEASE_BYTES: u64 = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum PackageDatabase {
    Arch,
    Rpm,
    Deb,
}

pub(crate) trait PlatformProbe: Send + Sync {
    fn current_executable(&self) -> Option<PathBuf>;
    fn ostree_booted(&self) -> bool;
    fn package_installed(&self, database: PackageDatabase) -> bool;
    fn trusted_executable(&self, path: &Path) -> bool;
    fn os_release(&self) -> Option<Vec<u8>>;
}

pub(crate) struct PackageTargetDetector {
    probe: Arc<dyn PlatformProbe>,
}

impl PackageTargetDetector {
    pub(crate) fn production() -> Self {
        Self {
            probe: Arc::new(SystemPlatformProbe),
        }
    }

    #[cfg(test)]
    pub(crate) fn injected(probe: Arc<dyn PlatformProbe>) -> Self {
        Self { probe }
    }

    pub(crate) fn detect(&self) -> PackageTarget {
        if self.probe.current_executable().as_deref() != Some(Path::new(CONTROL_BINARY))
            || !self.probe.trusted_executable(Path::new(PKEXEC))
        {
            return PackageTarget::Manual;
        }

        let arch = self.probe.package_installed(PackageDatabase::Arch);
        let rpm = self.probe.package_installed(PackageDatabase::Rpm);
        let deb = self.probe.package_installed(PackageDatabase::Deb);
        if usize::from(arch) + usize::from(rpm) + usize::from(deb) != 1 {
            return PackageTarget::Manual;
        }

        if self.probe.ostree_booted() {
            return if rpm
                && self.probe.trusted_executable(Path::new(RPM_OSTREE))
                && let Some(fedora_major) = fedora_major(self.probe.os_release().as_deref())
            {
                PackageTarget::RpmOstree { fedora_major }
            } else {
                PackageTarget::Manual
            };
        }

        if arch && self.probe.trusted_executable(Path::new(PACMAN)) {
            return PackageTarget::Arch;
        }
        if deb && self.probe.trusted_executable(Path::new(APT_GET)) {
            return PackageTarget::Deb;
        }
        if rpm
            && (self.probe.trusted_executable(Path::new(DNF5))
                || self.probe.trusted_executable(Path::new(DNF)))
            && let Some(fedora_major) = fedora_major(self.probe.os_release().as_deref())
        {
            return PackageTarget::Rpm { fedora_major };
        }
        PackageTarget::Manual
    }
}

struct SystemPlatformProbe;

impl PlatformProbe for SystemPlatformProbe {
    fn current_executable(&self) -> Option<PathBuf> {
        env::current_exe()
            .ok()
            .and_then(|path| fs::canonicalize(path).ok())
    }

    fn ostree_booted(&self) -> bool {
        Path::new(OSTREE_BOOTED).exists()
    }

    fn package_installed(&self, database: PackageDatabase) -> bool {
        let (program, args): (&str, &[&str]) = match database {
            PackageDatabase::Arch => (PACMAN, &["-Q", "overcrow-bin"]),
            PackageDatabase::Rpm => (RPM, &["-q", "overcrow"]),
            PackageDatabase::Deb => (DPKG_QUERY, &["--status", "overcrow"]),
        };
        self.trusted_executable(Path::new(program))
            && run_bounded_command(program, args, PROBE_TIMEOUT).is_ok()
    }

    fn trusted_executable(&self, path: &Path) -> bool {
        trusted_root_executable(path)
    }

    fn os_release(&self) -> Option<Vec<u8>> {
        [
            Path::new("/run/host/os-release"),
            Path::new("/etc/os-release"),
            Path::new("/usr/lib/os-release"),
        ]
        .into_iter()
        .find_map(read_bounded_file)
    }
}

fn trusted_root_executable(path: &Path) -> bool {
    if !is_normal_absolute_path(path) {
        return false;
    }
    let link_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if link_metadata.uid() != 0 {
        return false;
    }
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(_) => return false,
    };
    let metadata = match fs::metadata(canonical) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    metadata.file_type().is_file()
        && metadata.uid() == 0
        && metadata.permissions().mode() & 0o111 != 0
        && metadata.permissions().mode() & 0o022 == 0
}

fn read_bounded_file(path: &Path) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_OS_RELEASE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_OS_RELEASE_BYTES).then_some(bytes)
}

fn fedora_major(os_release: Option<&[u8]>) -> Option<u16> {
    let bytes = os_release?;
    let id = os_release_value(bytes, b"ID")?;
    if !matches!(id.as_str(), "fedora" | "bazzite") {
        return None;
    }
    let version = os_release_value(bytes, b"VERSION_ID")?;
    if version.is_empty() || version.len() > 3 || !version.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    version.parse().ok()
}

fn os_release_value(bytes: &[u8], key: &[u8]) -> Option<String> {
    if bytes.len() as u64 > MAX_OS_RELEASE_BYTES || bytes.contains(&0) {
        return None;
    }
    for line in bytes.split(|byte| *byte == b'\n') {
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (name, value) = line.split_at(separator);
        let value = value.get(1..)?;
        if name != key {
            continue;
        }
        let value = std::str::from_utf8(value).ok()?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        if value.is_empty()
            || value.len() > 64
            || value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return None;
        }
        return Some(value.to_ascii_lowercase());
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InstallerExit {
    Success,
    AuthorizationCancelled,
    Failed,
}

pub(crate) trait InstallerRunner: Send + Sync {
    fn run(&self, program: &Path, args: &[OsString]) -> Result<InstallerExit, UpdateError>;
}

struct SystemInstallerRunner;

impl InstallerRunner for SystemInstallerRunner {
    fn run(&self, program: &Path, args: &[OsString]) -> Result<InstallerExit, UpdateError> {
        let status =
            run_bounded_command_status(program, args, INSTALL_TIMEOUT).map_err(|error| {
                if error.contains("timed out") {
                    UpdateError::Timeout
                } else {
                    UpdateError::Installation
                }
            })?;
        Ok(match status.code() {
            Some(0) => InstallerExit::Success,
            Some(126 | 127) => InstallerExit::AuthorizationCancelled,
            _ => InstallerExit::Failed,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InstallOutcome {
    Installed,
    RestartRequired,
}

pub(crate) struct InstallerPlan {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
    outcome: InstallOutcome,
}

pub(crate) struct NativePackageInstaller {
    runner: Arc<dyn InstallerRunner>,
}

pub(crate) trait PackageInstaller: Send + Sync {
    fn install(
        &self,
        package: &VerifiedPackage,
        target: PackageTarget,
    ) -> Result<InstallOutcome, UpdateError>;
}

impl NativePackageInstaller {
    pub(crate) fn production() -> Self {
        Self {
            runner: Arc::new(SystemInstallerRunner),
        }
    }

    #[cfg(test)]
    pub(crate) fn injected(runner: Arc<dyn InstallerRunner>) -> Self {
        Self { runner }
    }

    pub(crate) fn install(
        &self,
        package: &VerifiedPackage,
        target: PackageTarget,
    ) -> Result<InstallOutcome, UpdateError> {
        package.validate()?;
        let plan = installer_plan(target, package.path(), &SystemPlatformProbe)?;
        self.run_plan(plan)
    }

    pub(crate) fn run_plan(&self, plan: InstallerPlan) -> Result<InstallOutcome, UpdateError> {
        match self.runner.run(&plan.program, &plan.args)? {
            InstallerExit::Success => Ok(plan.outcome),
            InstallerExit::AuthorizationCancelled => Err(UpdateError::AuthorizationCancelled),
            InstallerExit::Failed => Err(UpdateError::Installation),
        }
    }

    #[cfg(test)]
    pub(crate) fn plan_for_tests(
        target: PackageTarget,
        package: &Path,
        dnf5_available: bool,
    ) -> Result<InstallerPlan, UpdateError> {
        struct TestTrust(bool);
        impl PlatformProbe for TestTrust {
            fn current_executable(&self) -> Option<PathBuf> {
                None
            }
            fn ostree_booted(&self) -> bool {
                false
            }
            fn package_installed(&self, _database: PackageDatabase) -> bool {
                false
            }
            fn trusted_executable(&self, path: &Path) -> bool {
                path != Path::new(DNF5) || self.0
            }
            fn os_release(&self) -> Option<Vec<u8>> {
                None
            }
        }
        installer_plan(target, package, &TestTrust(dnf5_available))
    }
}

impl PackageInstaller for NativePackageInstaller {
    fn install(
        &self,
        package: &VerifiedPackage,
        target: PackageTarget,
    ) -> Result<InstallOutcome, UpdateError> {
        Self::install(self, package, target)
    }
}

fn installer_plan(
    target: PackageTarget,
    package: &Path,
    probe: &dyn PlatformProbe,
) -> Result<InstallerPlan, UpdateError> {
    if !is_normal_absolute_path(package) || !probe.trusted_executable(Path::new(PKEXEC)) {
        return Err(UpdateError::Installation);
    }
    let (manager, manager_args, outcome): (&str, &[&str], InstallOutcome) = match target {
        PackageTarget::Arch => (PACMAN, &["--noconfirm", "-U"], InstallOutcome::Installed),
        PackageTarget::Rpm { .. } => {
            let manager = if probe.trusted_executable(Path::new(DNF5)) {
                DNF5
            } else {
                DNF
            };
            (manager, &["install", "-y"], InstallOutcome::Installed)
        }
        PackageTarget::Deb => (APT_GET, &["install", "-y"], InstallOutcome::Installed),
        PackageTarget::RpmOstree { .. } => {
            (RPM_OSTREE, &["install"], InstallOutcome::RestartRequired)
        }
        PackageTarget::Manual => return Err(UpdateError::Installation),
    };
    if !probe.trusted_executable(Path::new(manager)) {
        return Err(UpdateError::Installation);
    }
    let mut args = Vec::with_capacity(manager_args.len() + 2);
    args.push(OsString::from(manager));
    args.extend(manager_args.iter().map(OsString::from));
    args.push(package.as_os_str().to_owned());
    Ok(InstallerPlan {
        program: PathBuf::from(PKEXEC),
        args,
        outcome,
    })
}
