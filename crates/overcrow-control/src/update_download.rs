use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        prelude::OsStrExt,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::{
    update_release::MAX_PACKAGE_BYTES,
    updates::{UpdateAsset, UpdateError},
};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_REDIRECTS: usize = 2;
const DOWNLOAD_USER_AGENT: &str = concat!(
    "PlayerVox-OverCrow/",
    env!("CARGO_PKG_VERSION"),
    " (update download)"
);
const APPROVED_ASSET_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];
const READ_BUFFER_BYTES: usize = 32 * 1024;
const MAX_CACHE_ENTRIES: usize = 32;

pub(crate) trait PackageDownloader: Send + Sync {
    fn download(&self, asset: &UpdateAsset) -> Result<VerifiedPackage, UpdateError>;
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedPackage {
    path: PathBuf,
    size: u64,
    sha256: [u8; 32],
}

impl VerifiedPackage {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn validate(&self) -> Result<(), UpdateError> {
        validate_regular_user_file(&self.path, self.size)?;
        let mut file = File::open(&self.path).map_err(|_| UpdateError::UnsafeCache)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; READ_BUFFER_BYTES];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| UpdateError::UnsafeCache)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let digest: [u8; 32] = hasher.finalize().into();
        if digest != self.sha256 {
            return Err(UpdateError::DigestMismatch);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn for_tests(path: PathBuf, size: u64, sha256: [u8; 32]) -> Self {
        Self { path, size, sha256 }
    }
}

pub(crate) struct GithubPackageDownloader {
    cache_root: PathBuf,
    agent: ureq::Agent,
    timeout: Duration,
    trust: DownloadTrust,
    require_managed_layout: bool,
}

struct DownloadTrust {
    https_only: bool,
    approved_hosts: &'static [&'static str],
}

impl GithubPackageDownloader {
    pub(crate) fn production(cache_root: PathBuf) -> Self {
        Self::new(
            cache_root,
            DOWNLOAD_TIMEOUT,
            DownloadTrust {
                https_only: true,
                approved_hosts: APPROVED_ASSET_HOSTS,
            },
            true,
        )
    }

    fn new(
        cache_root: PathBuf,
        timeout: Duration,
        trust: DownloadTrust,
        require_managed_layout: bool,
    ) -> Self {
        let agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .https_only(trust.https_only)
            .proxy(None)
            .build()
            .into();
        Self {
            cache_root,
            agent,
            timeout,
            trust,
            require_managed_layout,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(cache_root: PathBuf, timeout: Duration) -> Self {
        Self::new(
            cache_root,
            timeout,
            DownloadTrust {
                https_only: false,
                approved_hosts: &["127.0.0.1"],
            },
            false,
        )
    }

    fn download_inner(&self, asset: &UpdateAsset) -> Result<VerifiedPackage, UpdateError> {
        validate_asset_name(&asset.name)?;
        if asset.size == 0 || asset.size > MAX_PACKAGE_BYTES {
            return Err(UpdateError::DownloadTooLarge);
        }
        ensure_private_cache(&self.cache_root, self.require_managed_layout)?;
        clear_cache(&self.cache_root)?;

        let started = Instant::now();
        let mut url = Url::parse(&asset.api_url).map_err(|_| UpdateError::UntrustedRedirect)?;
        for redirects in 0..=MAX_REDIRECTS {
            self.trust.validate(&url)?;
            let remaining = self
                .timeout
                .checked_sub(started.elapsed())
                .filter(|remaining| !remaining.is_zero())
                .ok_or(UpdateError::Timeout)?;
            let mut response = self
                .agent
                .get(url.as_str())
                .config()
                .timeout_global(Some(remaining))
                .build()
                .header("Accept", "application/octet-stream")
                .header("User-Agent", DOWNLOAD_USER_AGENT)
                .call()
                .map_err(map_network_error)?;
            if response.status().is_redirection() {
                if redirects == MAX_REDIRECTS {
                    return Err(UpdateError::UntrustedRedirect);
                }
                let location = response
                    .headers()
                    .get("Location")
                    .and_then(|value| value.to_str().ok())
                    .ok_or(UpdateError::UntrustedRedirect)?;
                url = Url::parse(location).map_err(|_| UpdateError::UntrustedRedirect)?;
                continue;
            }
            if response.status().as_u16() != 200 {
                return Err(UpdateError::Download);
            }
            let mut reader = response.body_mut().as_reader();
            return self.write_verified(&mut reader, asset);
        }
        Err(UpdateError::UntrustedRedirect)
    }

    fn write_verified(
        &self,
        reader: &mut dyn Read,
        asset: &UpdateAsset,
    ) -> Result<VerifiedPackage, UpdateError> {
        let temporary = self
            .cache_root
            .join(format!(".download-{}", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)
                .map_err(|_| UpdateError::UnsafeCache)?;
            let mut hasher = Sha256::new();
            let mut written = 0_u64;
            let mut buffer = [0_u8; READ_BUFFER_BYTES];
            loop {
                let count = reader.read(&mut buffer).map_err(map_read_error)?;
                if count == 0 {
                    break;
                }
                written = written
                    .checked_add(count as u64)
                    .filter(|value| *value <= asset.size && *value <= MAX_PACKAGE_BYTES)
                    .ok_or(UpdateError::DownloadTooLarge)?;
                hasher.update(&buffer[..count]);
                file.write_all(&buffer[..count])
                    .map_err(|_| UpdateError::Download)?;
            }
            if written != asset.size {
                return Err(UpdateError::Download);
            }
            let digest: [u8; 32] = hasher.finalize().into();
            if digest != asset.sha256 {
                return Err(UpdateError::DigestMismatch);
            }
            file.sync_all().map_err(|_| UpdateError::Download)?;
            drop(file);

            let final_path = self.cache_root.join(&asset.name);
            fs::rename(&temporary, &final_path).map_err(|_| UpdateError::UnsafeCache)?;
            let directory = File::open(&self.cache_root).map_err(|_| UpdateError::UnsafeCache)?;
            directory.sync_all().map_err(|_| UpdateError::UnsafeCache)?;
            let package = VerifiedPackage {
                path: final_path,
                size: asset.size,
                sha256: asset.sha256,
            };
            package.validate()?;
            Ok(package)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

impl PackageDownloader for GithubPackageDownloader {
    fn download(&self, asset: &UpdateAsset) -> Result<VerifiedPackage, UpdateError> {
        self.download_inner(asset)
    }
}

impl DownloadTrust {
    fn validate(&self, url: &Url) -> Result<(), UpdateError> {
        let scheme_allowed = if self.https_only {
            url.scheme() == "https"
        } else {
            matches!(url.scheme(), "http" | "https")
        };
        if !scheme_allowed
            || !self
                .approved_hosts
                .contains(&url.host_str().unwrap_or_default())
            || url.username() != ""
            || url.password().is_some()
        {
            return Err(UpdateError::UntrustedRedirect);
        }
        Ok(())
    }
}

fn ensure_private_cache(path: &Path, require_managed_layout: bool) -> Result<(), UpdateError> {
    if !path.is_absolute() {
        return Err(UpdateError::UnsafeCache);
    }
    match fs::symlink_metadata(path) {
        Ok(_) => return ensure_private_directory(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(UpdateError::UnsafeCache),
    }
    let application_root = path.parent().ok_or(UpdateError::UnsafeCache)?;
    if require_managed_layout
        && (path.file_name() != Some(std::ffi::OsStr::new("updates"))
            || application_root.file_name() != Some(std::ffi::OsStr::new("overcrow")))
    {
        return Err(UpdateError::UnsafeCache);
    }
    let cache_base = application_root.parent().ok_or(UpdateError::UnsafeCache)?;
    validate_existing_directory_path(cache_base)?;
    let base_metadata = fs::symlink_metadata(cache_base).map_err(|_| UpdateError::UnsafeCache)?;
    if base_metadata.uid() != current_uid() {
        return Err(UpdateError::UnsafeCache);
    }
    ensure_private_directory(application_root)?;
    ensure_private_directory(path)?;
    Ok(())
}

fn validate_existing_directory_path(path: &Path) -> Result<(), UpdateError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|_| UpdateError::UnsafeCache)?;
        if !metadata.file_type().is_dir() {
            return Err(UpdateError::UnsafeCache);
        }
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), UpdateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.uid() != current_uid() {
                return Err(UpdateError::UnsafeCache);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|_| UpdateError::UnsafeCache)?;
        }
        Err(_) => return Err(UpdateError::UnsafeCache),
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| UpdateError::UnsafeCache)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| UpdateError::UnsafeCache)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != current_uid()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(UpdateError::UnsafeCache);
    }
    Ok(())
}

fn clear_cache(path: &Path) -> Result<(), UpdateError> {
    let mut removable = Vec::new();
    for (index, entry) in fs::read_dir(path)
        .map_err(|_| UpdateError::UnsafeCache)?
        .enumerate()
    {
        if index >= MAX_CACHE_ENTRIES {
            return Err(UpdateError::UnsafeCache);
        }
        let entry = entry.map_err(|_| UpdateError::UnsafeCache)?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path).map_err(|_| UpdateError::UnsafeCache)?;
        let name = entry.file_name();
        let name = name.as_bytes();
        if !metadata.file_type().is_file()
            || metadata.uid() != current_uid()
            || (!name.starts_with(b".download-") && !name.starts_with(b"overcrow"))
        {
            return Err(UpdateError::UnsafeCache);
        }
        removable.push(entry_path);
    }
    for entry in removable {
        fs::remove_file(entry).map_err(|_| UpdateError::UnsafeCache)?;
    }
    Ok(())
}

fn validate_asset_name(name: &str) -> Result<(), UpdateError> {
    let path = Path::new(name);
    if name.is_empty()
        || name.len() > 256
        || name.as_bytes().contains(&b'/')
        || path.file_name().and_then(|value| value.to_str()) != Some(name)
    {
        return Err(UpdateError::UnsafeCache);
    }
    Ok(())
}

fn validate_regular_user_file(path: &Path, expected_size: u64) -> Result<(), UpdateError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| UpdateError::UnsafeCache)?;
    if !metadata.file_type().is_file()
        || metadata.uid() != current_uid()
        || metadata.len() != expected_size
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(UpdateError::UnsafeCache);
    }
    Ok(())
}

fn current_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and does not retain pointers.
    unsafe { libc::geteuid() }
}

fn map_network_error(error: ureq::Error) -> UpdateError {
    match error {
        ureq::Error::Timeout(_) => UpdateError::Timeout,
        _ => UpdateError::Download,
    }
}

fn map_read_error(error: std::io::Error) -> UpdateError {
    if error.kind() == std::io::ErrorKind::TimedOut {
        UpdateError::Timeout
    } else {
        UpdateError::Download
    }
}
