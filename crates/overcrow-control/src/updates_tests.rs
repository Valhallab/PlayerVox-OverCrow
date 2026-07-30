use semver::Version;

use crate::{
    update_release::{arch_asset_name, deb_asset_name, rpm_asset_name, select_release},
    updates::{PackageTarget, ReleaseChannel},
};

const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn asset(name: &str, digest: Option<&str>) -> String {
    let digest = digest
        .map(|value| format!(r#""{value}""#))
        .unwrap_or_else(|| "null".to_owned());
    format!(
        r#"{{
            "name": "{name}",
            "url": "https://api.github.com/repos/Valhallab/PlayerVox-OverCrow/releases/assets/42",
            "size": 1024,
            "digest": {digest}
        }}"#
    )
}

fn release(
    tag: &str,
    draft: bool,
    prerelease: bool,
    assets: impl IntoIterator<Item = String>,
) -> String {
    format!(
        r#"{{
            "tag_name": "{tag}",
            "html_url": "https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/{tag}",
            "draft": {draft},
            "prerelease": {prerelease},
            "assets": [{}]
        }}"#,
        assets.into_iter().collect::<Vec<_>>().join(",")
    )
}

fn index(releases: impl IntoIterator<Item = String>) -> Vec<u8> {
    format!("[{}]", releases.into_iter().collect::<Vec<_>>().join(",")).into_bytes()
}

#[test]
fn pre_alpha_selects_the_newest_published_compatible_release() {
    let current = Version::parse("0.1.0-pre-alpha.4").expect("current version");
    let body = index([
        release(
            "v0.1.0-pre-alpha.5",
            false,
            true,
            [asset(
                "overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst",
                Some(DIGEST),
            )],
        ),
        release(
            "v0.1.0-pre-alpha.7",
            true,
            true,
            [asset(
                "overcrow-bin-0.1.0prealpha7-1-x86_64.pkg.tar.zst",
                Some(DIGEST),
            )],
        ),
        release(
            "v0.1.0-pre-alpha.3",
            false,
            true,
            [asset(
                "overcrow-bin-0.1.0prealpha3-1-x86_64.pkg.tar.zst",
                Some(DIGEST),
            )],
        ),
    ]);

    let selected = select_release(
        &body,
        &current,
        ReleaseChannel::PreRelease,
        PackageTarget::Arch,
    )
    .expect("valid release index")
    .expect("new release");

    assert_eq!(
        selected.version,
        Version::parse("0.1.0-pre-alpha.5").unwrap()
    );
    assert_eq!(
        selected.asset.expect("compatible asset").name,
        "overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst"
    );
}

#[test]
fn stable_builds_ignore_pre_releases_but_pre_releases_accept_stable() {
    let prerelease = index([release("v1.1.0-rc.1", false, true, [])]);
    assert!(
        select_release(
            &prerelease,
            &Version::parse("1.0.0").unwrap(),
            ReleaseChannel::Stable,
            PackageTarget::Manual,
        )
        .expect("valid release index")
        .is_none()
    );

    let stable = index([release("v1.0.0", false, false, [])]);
    let selected = select_release(
        &stable,
        &Version::parse("0.9.0-pre.1").unwrap(),
        ReleaseChannel::PreRelease,
        PackageTarget::Manual,
    )
    .expect("valid release index")
    .expect("stable upgrade");
    assert_eq!(selected.version, Version::parse("1.0.0").unwrap());
}

#[test]
fn package_asset_names_match_the_release_build_contract() {
    let version = Version::parse("0.1.0-pre-alpha.5").unwrap();
    assert_eq!(
        arch_asset_name(&version),
        "overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst"
    );
    assert_eq!(
        rpm_asset_name(&version, 42),
        "overcrow-0.1.0.pre_alpha.5-1.fc42.x86_64.rpm"
    );
    assert_eq!(
        deb_asset_name(&version),
        "overcrow_0.1.0~pre.alpha.5-1_amd64.deb"
    );
}

#[test]
fn malformed_or_unverified_assets_are_never_installable() {
    let current = Version::parse("0.1.0-pre-alpha.4").unwrap();
    let body = index([release(
        "v0.1.0-pre-alpha.5",
        false,
        true,
        [
            asset(
                "overcrow-bin-0.1.0prealpha5-1-aarch64.pkg.tar.zst",
                Some(DIGEST),
            ),
            asset("overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst", None),
        ],
    )]);

    let selected = select_release(
        &body,
        &current,
        ReleaseChannel::PreRelease,
        PackageTarget::Arch,
    )
    .expect("release metadata remains usable for manual fallback")
    .expect("new release");
    assert!(selected.asset.is_none());
}

#[test]
fn release_and_asset_entry_limits_fail_closed() {
    let current = Version::parse("0.1.0-pre-alpha.4").unwrap();
    let too_many_releases =
        index((0..21).map(|minor| release(&format!("v0.2.{minor}"), false, false, [])));
    assert!(
        select_release(
            &too_many_releases,
            &current,
            ReleaseChannel::PreRelease,
            PackageTarget::Manual,
        )
        .is_err()
    );

    let too_many_assets = index([release(
        "v0.1.0-pre-alpha.5",
        false,
        true,
        (0..21).map(|index| asset(&format!("ignored-{index}"), Some(DIGEST))),
    )]);
    assert!(
        select_release(
            &too_many_assets,
            &current,
            ReleaseChannel::PreRelease,
            PackageTarget::Arch,
        )
        .is_err()
    );
}

mod download {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        os::unix::fs::{PermissionsExt, symlink},
        sync::Arc,
        thread,
        time::Duration,
    };

    use semver::Version;
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use crate::{
        update_download::{GithubPackageDownloader, PackageDownloader},
        update_release::{GithubReleaseSource, ReleaseQuery, ReleaseSource},
        updates::{PackageTarget, ReleaseChannel, UpdateAsset, UpdateError},
    };

    fn sha256(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn test_server(
        requests: usize,
        response: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener");
        let address = listener.local_addr().expect("listener address");
        let response = Arc::new(response);
        let worker = thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().expect("test request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .expect("read timeout");
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..count]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                stream
                    .write_all(&response(path))
                    .expect("write test response");
            }
        });
        (format!("http://{address}"), worker)
    }

    fn http_ok(body: &[u8], content_type: &str) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn redirect_server(package: &'static [u8]) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("redirect listener");
        let address = listener.local_addr().expect("redirect address");
        let base = format!("http://{address}");
        let location = format!("{base}/asset");
        let worker = thread::spawn(move || {
            for response in [
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .into_bytes(),
                http_ok(package, "application/octet-stream"),
            ] {
                let (mut stream, _) = listener.accept().expect("redirect request");
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request).expect("read redirect request");
                stream.write_all(&response).expect("write redirect response");
            }
        });
        (base, worker)
    }

    fn asset(url: String, bytes: &[u8]) -> UpdateAsset {
        UpdateAsset {
            name: "overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst".to_owned(),
            api_url: url,
            size: bytes.len() as u64,
            sha256: sha256(bytes),
        }
    }

    #[test]
    fn github_release_source_reads_one_bounded_index() {
        let body = super::index([super::release(
            "v0.1.0-pre-alpha.5",
            false,
            true,
            [super::asset(
                "overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst",
                Some(super::DIGEST),
            )],
        )]);
        let response_body = body.clone();
        let (base, worker) = test_server(1, move |_| http_ok(&response_body, "application/json"));
        let source =
            GithubReleaseSource::for_tests(format!("{base}/releases"), Duration::from_secs(2));

        let selected = source
            .latest(ReleaseQuery {
                current: Version::parse("0.1.0-pre-alpha.4").unwrap(),
                channel: ReleaseChannel::PreRelease,
                target: PackageTarget::Arch,
            })
            .expect("release query")
            .expect("new release");
        assert_eq!(
            selected.version,
            Version::parse("0.1.0-pre-alpha.5").unwrap()
        );
        worker.join().expect("release server");
    }

    #[test]
    fn verified_download_is_published_atomically() {
        let package = b"verified package";
        let response_body = package.to_vec();
        let (base, worker) = test_server(1, move |_| {
            http_ok(&response_body, "application/octet-stream")
        });
        let cache = tempdir().expect("cache");
        let downloader =
            GithubPackageDownloader::for_tests(cache.path().to_path_buf(), Duration::from_secs(2));

        let verified = downloader
            .download(&asset(format!("{base}/asset"), package))
            .expect("verified package");
        assert_eq!(fs::read(verified.path()).unwrap(), package);
        assert_eq!(
            fs::read_dir(cache.path())
                .expect("cache entries")
                .filter_map(Result::ok)
                .count(),
            1
        );
        worker.join().expect("package server");
    }

    #[test]
    fn a_missing_application_cache_is_created_privately() {
        let package = b"verified package";
        let response_body = package.to_vec();
        let (base, worker) = test_server(1, move |_| {
            http_ok(&response_body, "application/octet-stream")
        });
        let parent = tempdir().expect("cache parent");
        let cache = parent.path().join("overcrow").join("updates");
        let downloader = GithubPackageDownloader::for_tests(cache.clone(), Duration::from_secs(2));

        downloader
            .download(&asset(format!("{base}/asset"), package))
            .expect("verified package");
        let metadata = fs::metadata(&cache).expect("private cache directory");
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            0o700,
            "the application cache must not be readable by other users"
        );
        worker.join().expect("package server");
    }

    #[test]
    fn digest_mismatch_never_publishes_a_cache_entry() {
        let package = b"tampered package";
        let response_body = package.to_vec();
        let (base, worker) = test_server(1, move |_| {
            http_ok(&response_body, "application/octet-stream")
        });
        let cache = tempdir().expect("cache");
        let downloader =
            GithubPackageDownloader::for_tests(cache.path().to_path_buf(), Duration::from_secs(2));
        let mut expected = asset(format!("{base}/asset"), package);
        expected.sha256 = sha256(b"expected package");

        assert_eq!(
            downloader.download(&expected).unwrap_err(),
            UpdateError::DigestMismatch
        );
        assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
        worker.join().expect("package server");
    }

    #[test]
    fn redirect_hosts_are_validated_before_the_next_request() {
        let (base, worker) = test_server(1, |_| {
            b"HTTP/1.1 302 Found\r\nLocation: https://example.com/package\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        });
        let cache = tempdir().expect("cache");
        let downloader =
            GithubPackageDownloader::for_tests(cache.path().to_path_buf(), Duration::from_secs(2));
        let expected = asset(format!("{base}/redirect"), b"package");

        assert_eq!(
            downloader.download(&expected).unwrap_err(),
            UpdateError::UntrustedRedirect
        );
        worker.join().expect("redirect server");
    }

    #[test]
    fn one_approved_redirect_can_publish_a_verified_package() {
        let package = b"redirected package";
        let (base, worker) = redirect_server(package);
        let cache = tempdir().expect("cache");
        let downloader =
            GithubPackageDownloader::for_tests(cache.path().to_path_buf(), Duration::from_secs(2));

        let verified = downloader
            .download(&asset(format!("{base}/redirect"), package))
            .expect("redirected verified package");
        assert_eq!(fs::read(verified.path()).unwrap(), package);
        worker.join().expect("redirect server");
    }

    #[test]
    fn symlinked_cache_roots_fail_closed() {
        let parent = tempdir().expect("cache parent");
        let target = parent.path().join("target");
        fs::create_dir(&target).expect("target directory");
        let cache = parent.path().join("updates");
        symlink(&target, &cache).expect("cache symlink");
        let downloader = GithubPackageDownloader::for_tests(cache, Duration::from_millis(100));

        assert_eq!(
            downloader
                .download(&asset("http://127.0.0.1:9/unused".to_owned(), b"package"))
                .unwrap_err(),
            UpdateError::UnsafeCache
        );
    }
}

mod install {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use crate::{
        update_install::{
            InstallOutcome, InstallerExit, InstallerRunner, NativePackageInstaller,
            PackageDatabase, PackageTargetDetector, PlatformProbe,
        },
        updates::{PackageTarget, UpdateError},
    };

    #[derive(Default)]
    struct FakePlatform {
        current_executable: Option<PathBuf>,
        ostree_booted: bool,
        installed: BTreeSet<PackageDatabase>,
        trusted: BTreeSet<PathBuf>,
        os_release: Option<Vec<u8>>,
    }

    impl PlatformProbe for FakePlatform {
        fn current_executable(&self) -> Option<PathBuf> {
            self.current_executable.clone()
        }

        fn ostree_booted(&self) -> bool {
            self.ostree_booted
        }

        fn package_installed(&self, database: PackageDatabase) -> bool {
            self.installed.contains(&database)
        }

        fn trusted_executable(&self, path: &Path) -> bool {
            self.trusted.contains(path)
        }

        fn os_release(&self) -> Option<Vec<u8>> {
            self.os_release.clone()
        }
    }

    fn base_platform() -> FakePlatform {
        FakePlatform {
            current_executable: Some(PathBuf::from("/usr/bin/overcrow-control")),
            trusted: [PathBuf::from("/usr/bin/pkexec")].into_iter().collect(),
            ..FakePlatform::default()
        }
    }

    #[test]
    fn package_detection_covers_all_reviewed_channels_and_ostree_precedence() {
        let mut arch = base_platform();
        arch.installed.insert(PackageDatabase::Arch);
        arch.trusted.insert(PathBuf::from("/usr/bin/pacman"));
        assert_eq!(
            PackageTargetDetector::injected(Arc::new(arch)).detect(),
            PackageTarget::Arch
        );

        let mut deb = base_platform();
        deb.installed.insert(PackageDatabase::Deb);
        deb.trusted.insert(PathBuf::from("/usr/bin/apt-get"));
        assert_eq!(
            PackageTargetDetector::injected(Arc::new(deb)).detect(),
            PackageTarget::Deb
        );

        let mut rpm = base_platform();
        rpm.installed.insert(PackageDatabase::Rpm);
        rpm.trusted.insert(PathBuf::from("/usr/bin/dnf5"));
        rpm.os_release = Some(b"ID=fedora\nVERSION_ID=42\n".to_vec());
        assert_eq!(
            PackageTargetDetector::injected(Arc::new(rpm)).detect(),
            PackageTarget::Rpm { fedora_major: 42 }
        );

        let mut ostree = base_platform();
        ostree.ostree_booted = true;
        ostree.installed.insert(PackageDatabase::Rpm);
        ostree.trusted.insert(PathBuf::from("/usr/bin/rpm-ostree"));
        ostree.os_release = Some(b"ID=bazzite\nVERSION_ID=\"42\"\n".to_vec());
        assert_eq!(
            PackageTargetDetector::injected(Arc::new(ostree)).detect(),
            PackageTarget::RpmOstree { fedora_major: 42 }
        );
    }

    #[test]
    fn ambiguous_local_or_untrusted_installations_fall_back_to_manual() {
        let mut local = base_platform();
        local.current_executable = Some(PathBuf::from("/home/user/overcrow-control"));
        local.installed.insert(PackageDatabase::Arch);
        local.trusted.insert(PathBuf::from("/usr/bin/pacman"));
        assert_eq!(
            PackageTargetDetector::injected(Arc::new(local)).detect(),
            PackageTarget::Manual
        );

        let mut ambiguous = base_platform();
        ambiguous
            .installed
            .extend([PackageDatabase::Arch, PackageDatabase::Rpm]);
        ambiguous.trusted.extend([
            PathBuf::from("/usr/bin/pacman"),
            PathBuf::from("/usr/bin/dnf"),
        ]);
        ambiguous.os_release = Some(b"ID=fedora\nVERSION_ID=42\n".to_vec());
        assert_eq!(
            PackageTargetDetector::injected(Arc::new(ambiguous)).detect(),
            PackageTarget::Manual
        );

        let mut missing_pkexec = base_platform();
        missing_pkexec.trusted.clear();
        missing_pkexec.installed.insert(PackageDatabase::Deb);
        missing_pkexec
            .trusted
            .insert(PathBuf::from("/usr/bin/apt-get"));
        assert_eq!(
            PackageTargetDetector::injected(Arc::new(missing_pkexec)).detect(),
            PackageTarget::Manual
        );
    }

    #[derive(Default)]
    struct RecordingRunner {
        invocations: Mutex<Vec<(PathBuf, Vec<String>)>>,
        exits: Mutex<BTreeMap<PathBuf, InstallerExit>>,
    }

    impl InstallerRunner for RecordingRunner {
        fn run(
            &self,
            program: &Path,
            args: &[std::ffi::OsString],
        ) -> Result<InstallerExit, UpdateError> {
            self.invocations.lock().unwrap().push((
                program.to_path_buf(),
                args.iter()
                    .map(|value| value.to_string_lossy().into_owned())
                    .collect(),
            ));
            Ok(*self
                .exits
                .lock()
                .unwrap()
                .get(program)
                .unwrap_or(&InstallerExit::Success))
        }
    }

    #[test]
    fn installer_argv_is_fixed_for_every_package_target() {
        let package = Path::new("/home/user/.cache/overcrow/updates/package");
        let expected = [
            (
                PackageTarget::Arch,
                vec![
                    "/usr/bin/pacman",
                    "--noconfirm",
                    "-U",
                    package.to_str().unwrap(),
                ],
            ),
            (
                PackageTarget::Rpm { fedora_major: 42 },
                vec!["/usr/bin/dnf5", "install", "-y", package.to_str().unwrap()],
            ),
            (
                PackageTarget::Deb,
                vec![
                    "/usr/bin/apt-get",
                    "install",
                    "-y",
                    package.to_str().unwrap(),
                ],
            ),
            (
                PackageTarget::RpmOstree { fedora_major: 42 },
                vec!["/usr/bin/rpm-ostree", "install", package.to_str().unwrap()],
            ),
        ];

        for (target, args) in expected {
            let plan = NativePackageInstaller::plan_for_tests(target, package, true)
                .expect("reviewed installer plan");
            assert_eq!(plan.program, PathBuf::from("/usr/bin/pkexec"));
            assert_eq!(
                plan.args
                    .iter()
                    .map(|value| value.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                args
            );
        }
    }

    #[test]
    fn installer_maps_policykit_cancellation_and_ostree_success() {
        let runner = Arc::new(RecordingRunner::default());
        runner.exits.lock().unwrap().insert(
            PathBuf::from("/usr/bin/pkexec"),
            InstallerExit::AuthorizationCancelled,
        );
        let installer = NativePackageInstaller::injected(runner.clone());
        assert_eq!(
            installer
                .run_plan(
                    NativePackageInstaller::plan_for_tests(
                        PackageTarget::Arch,
                        Path::new("/tmp/package"),
                        true,
                    )
                    .unwrap(),
                )
                .unwrap_err(),
            UpdateError::AuthorizationCancelled
        );

        runner
            .exits
            .lock()
            .unwrap()
            .insert(PathBuf::from("/usr/bin/pkexec"), InstallerExit::Success);
        assert_eq!(
            installer
                .run_plan(
                    NativePackageInstaller::plan_for_tests(
                        PackageTarget::RpmOstree { fedora_major: 42 },
                        Path::new("/tmp/package"),
                        true,
                    )
                    .unwrap(),
                )
                .unwrap(),
            InstallOutcome::RestartRequired
        );
    }
}

mod controller {
    use std::{
        path::PathBuf,
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::{Duration, SystemTime},
    };

    use semver::Version;

    use crate::{
        update_download::{PackageDownloader, VerifiedPackage},
        update_install::{InstallOutcome, PackageInstaller},
        update_release::{ReleaseQuery, ReleaseSource},
        updates::{
            PackageTarget, UpdateAsset, UpdateCandidate, UpdateClock, UpdateController,
            UpdateError, UpdatePhase,
        },
    };

    struct FakeClock(Mutex<SystemTime>);

    impl FakeClock {
        fn new() -> Self {
            Self(Mutex::new(
                SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000),
            ))
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.0.lock().unwrap();
            *now = now.checked_add(duration).unwrap();
        }
    }

    impl UpdateClock for FakeClock {
        fn now(&self) -> SystemTime {
            *self.0.lock().unwrap()
        }
    }

    struct FakeReleaseSource {
        result: Mutex<Result<Option<UpdateCandidate>, UpdateError>>,
        calls: AtomicUsize,
    }

    impl ReleaseSource for FakeReleaseSource {
        fn latest(&self, _query: ReleaseQuery) -> Result<Option<UpdateCandidate>, UpdateError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.result.lock().unwrap().clone()
        }
    }

    struct FakeDownloader {
        result: Mutex<Result<VerifiedPackage, UpdateError>>,
        calls: AtomicUsize,
    }

    impl PackageDownloader for FakeDownloader {
        fn download(&self, _asset: &UpdateAsset) -> Result<VerifiedPackage, UpdateError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.result.lock().unwrap().clone()
        }
    }

    struct FakeInstaller {
        result: Mutex<Result<InstallOutcome, UpdateError>>,
        calls: AtomicUsize,
    }

    impl PackageInstaller for FakeInstaller {
        fn install(
            &self,
            _package: &VerifiedPackage,
            _target: PackageTarget,
        ) -> Result<InstallOutcome, UpdateError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            *self.result.lock().unwrap()
        }
    }

    fn candidate(with_asset: bool) -> UpdateCandidate {
        UpdateCandidate {
            version: Version::parse("0.1.0-pre-alpha.5").unwrap(),
            release_page:
                "https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/v0.1.0-pre-alpha.5"
                    .to_owned(),
            asset: with_asset.then_some(UpdateAsset {
                name: "overcrow-bin-0.1.0prealpha5-1-x86_64.pkg.tar.zst".to_owned(),
                api_url:
                    "https://api.github.com/repos/Valhallab/PlayerVox-OverCrow/releases/assets/42"
                        .to_owned(),
                size: 7,
                sha256: [7; 32],
            }),
        }
    }

    struct Fixture {
        controller: UpdateController,
        clock: Arc<FakeClock>,
        source: Arc<FakeReleaseSource>,
        downloader: Arc<FakeDownloader>,
        installer: Arc<FakeInstaller>,
    }

    fn fixture(target: PackageTarget, release: Option<UpdateCandidate>) -> Fixture {
        let clock = Arc::new(FakeClock::new());
        let source = Arc::new(FakeReleaseSource {
            result: Mutex::new(Ok(release)),
            calls: AtomicUsize::new(0),
        });
        let downloader = Arc::new(FakeDownloader {
            result: Mutex::new(Ok(VerifiedPackage::for_tests(
                PathBuf::from("/tmp/overcrow-update-test"),
                7,
                [7; 32],
            ))),
            calls: AtomicUsize::new(0),
        });
        let installer = Arc::new(FakeInstaller {
            result: Mutex::new(Ok(InstallOutcome::Installed)),
            calls: AtomicUsize::new(0),
        });
        let controller = UpdateController::injected(
            Version::parse("0.1.0-pre-alpha.4").unwrap(),
            target,
            source.clone(),
            downloader.clone(),
            installer.clone(),
            clock.clone(),
        );
        Fixture {
            controller,
            clock,
            source,
            downloader,
            installer,
        }
    }

    #[test]
    fn checks_publish_available_up_to_date_and_manual_states() {
        let available = fixture(PackageTarget::Arch, Some(candidate(true)));
        let observed = Mutex::new(Vec::new());
        let state = available
            .controller
            .check(false, |state| observed.lock().unwrap().push(state.phase))
            .expect("update check");
        assert_eq!(state.phase, UpdatePhase::Available);
        assert_eq!(
            observed.into_inner().unwrap(),
            [UpdatePhase::Checking, UpdatePhase::Available]
        );

        let current = fixture(PackageTarget::Arch, None);
        assert_eq!(
            current.controller.check(false, |_| {}).unwrap().phase,
            UpdatePhase::UpToDate
        );

        let manual = fixture(PackageTarget::Manual, Some(candidate(false)));
        assert_eq!(
            manual.controller.check(false, |_| {}).unwrap().phase,
            UpdatePhase::Manual
        );
    }

    #[test]
    fn automatic_and_manual_checks_use_separate_bounded_freshness_windows() {
        let fixture = fixture(PackageTarget::Arch, None);
        fixture.controller.check(false, |_| {}).unwrap();
        fixture.controller.check(false, |_| {}).unwrap();
        fixture.controller.check(true, |_| {}).unwrap();
        assert_eq!(fixture.source.calls.load(Ordering::Relaxed), 1);

        fixture.clock.advance(Duration::from_secs(61));
        fixture.controller.check(true, |_| {}).unwrap();
        assert_eq!(fixture.source.calls.load(Ordering::Relaxed), 2);

        fixture.clock.advance(Duration::from_secs(6 * 60 * 60));
        fixture.controller.check(false, |_| {}).unwrap();
        assert_eq!(fixture.source.calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn installation_stops_runtime_between_verification_and_privilege() {
        let fixture = fixture(PackageTarget::Arch, Some(candidate(true)));
        fixture.controller.check(false, |_| {}).unwrap();
        let ordering = Mutex::new(Vec::new());
        let final_state = fixture
            .controller
            .install_available_update(
                |state| ordering.lock().unwrap().push(state.phase),
                || {
                    ordering.lock().unwrap().push(UpdatePhase::Idle);
                    Ok(())
                },
            )
            .expect("install transaction");

        assert_eq!(final_state.phase, UpdatePhase::Installed);
        assert_eq!(fixture.downloader.calls.load(Ordering::Relaxed), 1);
        assert_eq!(fixture.installer.calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            ordering.into_inner().unwrap(),
            [
                UpdatePhase::Downloading,
                UpdatePhase::Idle,
                UpdatePhase::Installing,
                UpdatePhase::Installed,
            ]
        );
    }

    #[test]
    fn failed_runtime_stop_prevents_privileged_installation() {
        let fixture = fixture(PackageTarget::Arch, Some(candidate(true)));
        fixture.controller.check(false, |_| {}).unwrap();
        let state = fixture
            .controller
            .install_available_update(|_| {}, || Err(UpdateError::RuntimeStop))
            .expect("failure is represented in state");

        assert_eq!(state.phase, UpdatePhase::Failed);
        assert_eq!(fixture.installer.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn ostree_success_requires_a_system_restart() {
        let fixture = fixture(
            PackageTarget::RpmOstree { fedora_major: 42 },
            Some(candidate(true)),
        );
        *fixture.installer.result.lock().unwrap() = Ok(InstallOutcome::RestartRequired);
        fixture.controller.check(false, |_| {}).unwrap();

        assert_eq!(
            fixture
                .controller
                .install_available_update(|_| {}, || Ok(()))
                .unwrap()
                .phase,
            UpdatePhase::RestartRequired
        );
    }

    struct BlockingReleaseSource {
        state: Mutex<(bool, bool)>,
        changed: Condvar,
    }

    impl BlockingReleaseSource {
        fn wait_until_entered(&self) {
            let mut state = self.state.lock().unwrap();
            while !state.0 {
                state = self.changed.wait(state).unwrap();
            }
        }

        fn release(&self) {
            let mut state = self.state.lock().unwrap();
            state.1 = true;
            self.changed.notify_all();
        }
    }

    impl ReleaseSource for BlockingReleaseSource {
        fn latest(&self, _query: ReleaseQuery) -> Result<Option<UpdateCandidate>, UpdateError> {
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                state = self.changed.wait(state).unwrap();
            }
            Ok(None)
        }
    }

    #[test]
    fn concurrent_operations_are_rejected_without_replacing_visible_state() {
        let base = fixture(PackageTarget::Arch, None);
        let source = Arc::new(BlockingReleaseSource {
            state: Mutex::new((false, false)),
            changed: Condvar::new(),
        });
        let controller = Arc::new(UpdateController::injected(
            Version::parse("0.1.0-pre-alpha.4").unwrap(),
            PackageTarget::Arch,
            source.clone(),
            base.downloader,
            base.installer,
            base.clock,
        ));
        let worker_controller = controller.clone();
        let worker = thread::spawn(move || worker_controller.check(false, |_| {}));
        source.wait_until_entered();

        assert_eq!(controller.state().phase, UpdatePhase::Checking);
        assert_eq!(
            controller.check(true, |_| {}).unwrap_err(),
            UpdateError::OperationBusy
        );
        assert_eq!(controller.state().phase, UpdatePhase::Checking);

        source.release();
        assert_eq!(
            worker.join().expect("check worker").unwrap().phase,
            UpdatePhase::UpToDate
        );
    }

    #[test]
    fn a_new_check_invalidates_a_stale_install_candidate() {
        let fixture = fixture(PackageTarget::Arch, Some(candidate(true)));
        fixture.controller.check(false, |_| {}).unwrap();
        *fixture.source.result.lock().unwrap() = Ok(None);
        fixture.clock.advance(Duration::from_secs(61));
        fixture.controller.check(true, |_| {}).unwrap();

        assert_eq!(
            fixture
                .controller
                .install_available_update(|_| {}, || Ok(()))
                .unwrap_err(),
            UpdateError::Unavailable
        );
        assert_eq!(fixture.downloader.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn download_and_authorization_failures_are_visible_and_bounded() {
        let fixture = fixture(PackageTarget::Arch, Some(candidate(true)));
        fixture.controller.check(false, |_| {}).unwrap();
        *fixture.downloader.result.lock().unwrap() = Err(UpdateError::DigestMismatch);
        let state = fixture
            .controller
            .install_available_update(|_| {}, || Ok(()))
            .expect("download failure state");
        assert_eq!(state.phase, UpdatePhase::Failed);
        assert_eq!(
            state.error,
            Some(crate::updates::UpdateErrorCode::Verification)
        );
        assert_eq!(fixture.installer.calls.load(Ordering::Relaxed), 0);

        *fixture.downloader.result.lock().unwrap() = Ok(VerifiedPackage::for_tests(
            PathBuf::from("/tmp/overcrow-update-test"),
            7,
            [7; 32],
        ));
        *fixture.installer.result.lock().unwrap() = Err(UpdateError::AuthorizationCancelled);
        let state = fixture
            .controller
            .install_available_update(|_| {}, || Ok(()))
            .expect("authorization failure state");
        assert_eq!(state.phase, UpdatePhase::Failed);
        assert_eq!(
            state.error,
            Some(crate::updates::UpdateErrorCode::AuthorizationCancelled)
        );
    }
}
