use std::{
    env,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use chrono::{DateTime, SecondsFormat, Utc};
use overcrow_logging::{Component, EventLogger, LoggerRuntime};
use semver::Version;
use serde::Serialize;

use crate::{
    update_download::{GithubPackageDownloader, PackageDownloader},
    update_install::{
        InstallOutcome, NativePackageInstaller, PackageInstaller, PackageTargetDetector,
    },
    update_release::{GithubReleaseSource, ReleaseQuery, ReleaseSource},
};

pub const CONTROL_UPDATE_SCHEMA_VERSION: u32 = 1;
pub const RELEASES_PAGE: &str = "https://github.com/Valhallab/PlayerVox-OverCrow/releases";

const AUTOMATIC_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const MANUAL_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const OPEN_PAGE_TIMEOUT: Duration = Duration::from_secs(15);
const XDG_OPEN: &str = "/usr/bin/xdg-open";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    Installed,
    RestartRequired,
    Manual,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateInstallKind {
    Arch,
    Rpm,
    Deb,
    RpmOstree,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateErrorCode {
    Busy,
    Unavailable,
    Network,
    InvalidResponse,
    Download,
    Verification,
    RuntimeStop,
    AuthorizationCancelled,
    Installation,
    Timeout,
    OpenPage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlUpdateState {
    pub schema_version: u32,
    pub phase: UpdatePhase,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub install_kind: UpdateInstallKind,
    pub last_checked_at: Option<String>,
    pub error: Option<UpdateErrorCode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReleaseChannel {
    Stable,
    PreRelease,
}

impl ReleaseChannel {
    pub(crate) fn from_current(version: &Version) -> Self {
        if version.pre.is_empty() {
            Self::Stable
        } else {
            Self::PreRelease
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PackageTarget {
    Arch,
    Rpm { fedora_major: u16 },
    Deb,
    RpmOstree { fedora_major: u16 },
    Manual,
}

impl PackageTarget {
    pub(crate) const fn install_kind(self) -> UpdateInstallKind {
        match self {
            Self::Arch => UpdateInstallKind::Arch,
            Self::Rpm { .. } => UpdateInstallKind::Rpm,
            Self::Deb => UpdateInstallKind::Deb,
            Self::RpmOstree { .. } => UpdateInstallKind::RpmOstree,
            Self::Manual => UpdateInstallKind::Manual,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UpdateAsset {
    pub(crate) name: String,
    pub(crate) api_url: String,
    pub(crate) size: u64,
    pub(crate) sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UpdateCandidate {
    pub(crate) version: Version,
    pub(crate) release_page: String,
    pub(crate) asset: Option<UpdateAsset>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateError {
    InvalidResponse,
    OperationBusy,
    Unavailable,
    Network,
    Timeout,
    UntrustedRedirect,
    DownloadTooLarge,
    DigestMismatch,
    UnsafeCache,
    Download,
    RuntimeStop,
    AuthorizationCancelled,
    Installation,
    OpenPage,
}

impl UpdateError {
    pub const fn code(self) -> UpdateErrorCode {
        match self {
            Self::OperationBusy => UpdateErrorCode::Busy,
            Self::Unavailable => UpdateErrorCode::Unavailable,
            Self::Network | Self::UntrustedRedirect => UpdateErrorCode::Network,
            Self::InvalidResponse => UpdateErrorCode::InvalidResponse,
            Self::DownloadTooLarge | Self::Download => UpdateErrorCode::Download,
            Self::DigestMismatch | Self::UnsafeCache => UpdateErrorCode::Verification,
            Self::RuntimeStop => UpdateErrorCode::RuntimeStop,
            Self::AuthorizationCancelled => UpdateErrorCode::AuthorizationCancelled,
            Self::Installation => UpdateErrorCode::Installation,
            Self::Timeout => UpdateErrorCode::Timeout,
            Self::OpenPage => UpdateErrorCode::OpenPage,
        }
    }

    const fn category(self) -> &'static str {
        match self {
            Self::InvalidResponse => "invalid_response",
            Self::OperationBusy => "busy",
            Self::Unavailable => "unavailable",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::UntrustedRedirect => "untrusted_redirect",
            Self::DownloadTooLarge => "download_too_large",
            Self::DigestMismatch => "digest_mismatch",
            Self::UnsafeCache => "unsafe_cache",
            Self::Download => "download",
            Self::RuntimeStop => "runtime_stop",
            Self::AuthorizationCancelled => "authorization_cancelled",
            Self::Installation => "installation",
            Self::OpenPage => "open_page",
        }
    }
}

pub(crate) trait UpdateClock: Send + Sync {
    fn now(&self) -> SystemTime;
}

struct SystemClock;

impl UpdateClock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

struct UpdateInner {
    state: ControlUpdateState,
    candidate: Option<UpdateCandidate>,
    last_attempt: Option<SystemTime>,
    check_failure_active: bool,
    install_failure_active: bool,
}

pub struct UpdateController {
    current: Version,
    channel: ReleaseChannel,
    target: PackageTarget,
    inner: Mutex<UpdateInner>,
    operation_active: AtomicBool,
    release_source: Arc<dyn ReleaseSource>,
    downloader: Arc<dyn PackageDownloader>,
    installer: Arc<dyn PackageInstaller>,
    clock: Arc<dyn UpdateClock>,
    logger: EventLogger,
    // Keep the logging worker alive. Its receiver is Send but not Sync, so the
    // mutex also makes the controller safe to share with Tauri workers.
    _logger_runtime: Mutex<Option<LoggerRuntime>>,
}

impl UpdateController {
    pub fn production() -> Result<Self, UpdateError> {
        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| UpdateError::InvalidResponse)?;
        let target = PackageTargetDetector::production().detect();
        let logger_runtime = LoggerRuntime::start(Component::Control).ok();
        Ok(Self::new(
            current,
            target,
            Arc::new(GithubReleaseSource::default()),
            Arc::new(GithubPackageDownloader::production(update_cache_root())),
            Arc::new(NativePackageInstaller::production()),
            Arc::new(SystemClock),
            logger_runtime,
        ))
    }

    #[cfg(test)]
    pub(crate) fn injected(
        current: Version,
        target: PackageTarget,
        release_source: Arc<dyn ReleaseSource>,
        downloader: Arc<dyn PackageDownloader>,
        installer: Arc<dyn PackageInstaller>,
        clock: Arc<dyn UpdateClock>,
    ) -> Self {
        Self::new(
            current,
            target,
            release_source,
            downloader,
            installer,
            clock,
            None,
        )
    }

    fn new(
        current: Version,
        target: PackageTarget,
        release_source: Arc<dyn ReleaseSource>,
        downloader: Arc<dyn PackageDownloader>,
        installer: Arc<dyn PackageInstaller>,
        clock: Arc<dyn UpdateClock>,
        logger_runtime: Option<LoggerRuntime>,
    ) -> Self {
        let logger = logger_runtime
            .as_ref()
            .map(LoggerRuntime::logger)
            .unwrap_or_else(EventLogger::disabled);
        let state = ControlUpdateState {
            schema_version: CONTROL_UPDATE_SCHEMA_VERSION,
            phase: UpdatePhase::Idle,
            current_version: current.to_string(),
            latest_version: None,
            install_kind: target.install_kind(),
            last_checked_at: None,
            error: None,
        };
        Self {
            current: current.clone(),
            channel: ReleaseChannel::from_current(&current),
            target,
            inner: Mutex::new(UpdateInner {
                state,
                candidate: None,
                last_attempt: None,
                check_failure_active: false,
                install_failure_active: false,
            }),
            operation_active: AtomicBool::new(false),
            release_source,
            downloader,
            installer,
            clock,
            logger,
            _logger_runtime: Mutex::new(logger_runtime),
        }
    }

    pub fn state(&self) -> ControlUpdateState {
        self.inner().state.clone()
    }

    fn release_page(&self) -> String {
        self.inner()
            .candidate
            .as_ref()
            .map(|candidate| candidate.release_page.clone())
            .unwrap_or_else(|| RELEASES_PAGE.to_owned())
    }

    pub fn open_release_page(&self) -> Result<(), UpdateError> {
        let release_page = self.release_page();
        let mut child = Command::new(XDG_OPEN)
            .arg(release_page)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| UpdateError::OpenPage)?;
        let started = Instant::now();
        loop {
            let status = match child.try_wait() {
                Ok(status) => status,
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(UpdateError::OpenPage);
                }
            };
            match status {
                Some(status) if status.success() => return Ok(()),
                Some(_) => return Err(UpdateError::OpenPage),
                None if started.elapsed() < OPEN_PAGE_TIMEOUT => {
                    thread::sleep(Duration::from_millis(25));
                }
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(UpdateError::OpenPage);
                }
            }
        }
    }

    pub fn check<F>(&self, force: bool, mut observer: F) -> Result<ControlUpdateState, UpdateError>
    where
        F: FnMut(ControlUpdateState),
    {
        let _operation = self.begin_operation()?;
        let now = self.clock.now();
        let minimum_interval = if force {
            MANUAL_CHECK_INTERVAL
        } else {
            AUTOMATIC_CHECK_INTERVAL
        };
        {
            let inner = self.inner();
            if inner
                .last_attempt
                .and_then(|last| now.duration_since(last).ok())
                .is_some_and(|elapsed| elapsed < minimum_interval)
            {
                return Ok(inner.state.clone());
            }
        }

        self.publish(&mut observer, |inner| {
            inner.last_attempt = Some(now);
            inner.candidate = None;
            inner.state.phase = UpdatePhase::Checking;
            inner.state.latest_version = None;
            inner.state.error = None;
        });

        let result = self.release_source.latest(ReleaseQuery {
            current: self.current.clone(),
            channel: self.channel,
            target: self.target,
        });
        match result {
            Ok(candidate) => {
                let recovered = self.clear_failure(FailureStage::Check);
                let checked_at = timestamp(now);
                let final_state = self.publish(&mut observer, |inner| {
                    inner.state.last_checked_at = Some(checked_at);
                    inner.state.error = None;
                    match candidate {
                        Some(candidate) => {
                            inner.state.latest_version = Some(candidate.version.to_string());
                            inner.state.phase = if self.target == PackageTarget::Manual
                                || candidate.asset.is_none()
                            {
                                UpdatePhase::Manual
                            } else {
                                UpdatePhase::Available
                            };
                            inner.candidate = Some(candidate);
                        }
                        None => {
                            inner.state.phase = UpdatePhase::UpToDate;
                            inner.state.latest_version = None;
                            inner.candidate = None;
                        }
                    }
                });
                if recovered {
                    self.log_recovered(FailureStage::Check);
                }
                Ok(final_state)
            }
            Err(error) => {
                let final_state = self.publish_failure(&mut observer, error, FailureStage::Check);
                Ok(final_state)
            }
        }
    }

    pub fn install_available_update<F, S>(
        &self,
        mut observer: F,
        before_install: S,
    ) -> Result<ControlUpdateState, UpdateError>
    where
        F: FnMut(ControlUpdateState),
        S: FnOnce() -> Result<(), UpdateError>,
    {
        let _operation = self.begin_operation()?;
        let candidate = self
            .inner()
            .candidate
            .clone()
            .filter(|candidate| candidate.asset.is_some())
            .filter(|_| self.target != PackageTarget::Manual)
            .ok_or(UpdateError::Unavailable)?;
        let asset = candidate.asset.as_ref().ok_or(UpdateError::Unavailable)?;
        self.publish(&mut observer, |inner| {
            inner.state.phase = UpdatePhase::Downloading;
            inner.state.error = None;
        });

        let package = match self.downloader.download(asset) {
            Ok(package) => package,
            Err(error) => {
                return Ok(self.publish_failure(&mut observer, error, FailureStage::Install));
            }
        };
        if let Err(error) = before_install() {
            return Ok(self.publish_failure(&mut observer, error, FailureStage::Install));
        }
        self.publish(&mut observer, |inner| {
            inner.state.phase = UpdatePhase::Installing;
            inner.state.error = None;
        });
        match self.installer.install(&package, self.target) {
            Ok(outcome) => {
                let recovered = self.clear_failure(FailureStage::Install);
                let final_state = self.publish(&mut observer, |inner| {
                    inner.state.phase = match outcome {
                        InstallOutcome::Installed => UpdatePhase::Installed,
                        InstallOutcome::RestartRequired => UpdatePhase::RestartRequired,
                    };
                    inner.state.error = None;
                });
                if recovered {
                    self.log_recovered(FailureStage::Install);
                }
                Ok(final_state)
            }
            Err(error) => Ok(self.publish_failure(&mut observer, error, FailureStage::Install)),
        }
    }

    fn begin_operation(&self) -> Result<OperationGuard<'_>, UpdateError> {
        self.operation_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| UpdateError::OperationBusy)?;
        Ok(OperationGuard {
            active: &self.operation_active,
        })
    }

    fn publish<F>(
        &self,
        observer: &mut F,
        update: impl FnOnce(&mut UpdateInner),
    ) -> ControlUpdateState
    where
        F: FnMut(ControlUpdateState),
    {
        let state = {
            let mut inner = self.inner();
            update(&mut inner);
            inner.state.clone()
        };
        observer(state.clone());
        state
    }

    fn publish_failure<F>(
        &self,
        observer: &mut F,
        error: UpdateError,
        stage: FailureStage,
    ) -> ControlUpdateState
    where
        F: FnMut(ControlUpdateState),
    {
        let should_log = {
            let mut inner = self.inner();
            let flag = match stage {
                FailureStage::Check => &mut inner.check_failure_active,
                FailureStage::Install => &mut inner.install_failure_active,
            };
            let first = !*flag;
            *flag = true;
            first
        };
        if should_log {
            self.logger.warn(
                "control_update_failed",
                format_args!(
                    "stage={} category={} channel={}",
                    stage.name(),
                    error.category(),
                    self.channel.name()
                ),
            );
        }
        self.publish(observer, |inner| {
            inner.state.phase = UpdatePhase::Failed;
            inner.state.error = Some(error.code());
        })
    }

    fn clear_failure(&self, stage: FailureStage) -> bool {
        let mut inner = self.inner();
        let flag = match stage {
            FailureStage::Check => &mut inner.check_failure_active,
            FailureStage::Install => &mut inner.install_failure_active,
        };
        std::mem::take(flag)
    }

    fn log_recovered(&self, stage: FailureStage) {
        self.logger.info(
            "control_update_recovered",
            format_args!("stage={} channel={}", stage.name(), self.channel.name()),
        );
    }

    fn inner(&self) -> MutexGuard<'_, UpdateInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ReleaseChannel {
    const fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::PreRelease => "pre_release",
        }
    }
}

#[derive(Clone, Copy)]
enum FailureStage {
    Check,
    Install,
}

impl FailureStage {
    const fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Install => "install",
        }
    }
}

struct OperationGuard<'a> {
    active: &'a AtomicBool,
}

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

fn timestamp(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn update_cache_root() -> PathBuf {
    let root = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".cache"))
        })
        .unwrap_or_default();
    root.join("overcrow/updates")
}
