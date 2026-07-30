use std::{io::Read, time::Duration};

use semver::Version;
use serde::Deserialize;
use url::Url;

use crate::updates::{PackageTarget, ReleaseChannel, UpdateAsset, UpdateCandidate, UpdateError};

pub(crate) const MAX_RELEASE_INDEX_BYTES: usize = 256 * 1024;
pub(crate) const MAX_RELEASES: usize = 20;
pub(crate) const MAX_RELEASE_ASSETS: usize = 20;
pub(crate) const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;

const RELEASE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const RELEASE_USER_AGENT: &str = concat!(
    "PlayerVox-OverCrow/",
    env!("CARGO_PKG_VERSION"),
    " (update check)"
);
const RELEASES_ENDPOINT: &str =
    "https://api.github.com/repos/Valhallab/PlayerVox-OverCrow/releases?per_page=20";
const MAX_TAG_BYTES: usize = 128;
const MAX_RELEASE_URL_BYTES: usize = 512;
const MAX_ASSET_NAME_BYTES: usize = 256;
const MAX_ASSET_URL_BYTES: usize = 512;
const SHA256_PREFIX: &str = "sha256:";
const RELEASE_PAGE_PREFIX: &str = "https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/";
const ASSET_API_PREFIX: &str = "/repos/Valhallab/PlayerVox-OverCrow/releases/assets/";

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ReleaseQuery {
    pub(crate) current: Version,
    pub(crate) channel: ReleaseChannel,
    pub(crate) target: PackageTarget,
}

pub(crate) trait ReleaseSource: Send + Sync {
    fn latest(&self, query: ReleaseQuery) -> Result<Option<UpdateCandidate>, UpdateError>;
}

pub(crate) struct GithubReleaseSource {
    endpoint: String,
    agent: ureq::Agent,
}

impl Default for GithubReleaseSource {
    fn default() -> Self {
        Self::new(RELEASES_ENDPOINT, RELEASE_REQUEST_TIMEOUT, true)
    }
}

impl GithubReleaseSource {
    fn new(endpoint: &str, timeout: Duration, https_only: bool) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .https_only(https_only)
            .proxy(None)
            .build()
            .into();
        Self {
            endpoint: endpoint.to_owned(),
            agent,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(endpoint: String, timeout: Duration) -> Self {
        Self::new(&endpoint, timeout, false)
    }
}

impl ReleaseSource for GithubReleaseSource {
    fn latest(&self, query: ReleaseQuery) -> Result<Option<UpdateCandidate>, UpdateError> {
        let mut response = self
            .agent
            .get(&self.endpoint)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", RELEASE_USER_AGENT)
            .call()
            .map_err(map_network_error)?;
        if response.status().as_u16() != 200 {
            return Err(UpdateError::Network);
        }
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take((MAX_RELEASE_INDEX_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(map_read_error)?;
        if body.len() > MAX_RELEASE_INDEX_BYTES {
            return Err(UpdateError::InvalidResponse);
        }
        select_release(&body, &query.current, query.channel, query.target)
    }
}

fn map_network_error(error: ureq::Error) -> UpdateError {
    match error {
        ureq::Error::Timeout(_) => UpdateError::Timeout,
        _ => UpdateError::Network,
    }
}

fn map_read_error(error: std::io::Error) -> UpdateError {
    if error.kind() == std::io::ErrorKind::TimedOut {
        UpdateError::Timeout
    } else {
        UpdateError::Network
    }
}

pub(crate) fn select_release(
    body: &[u8],
    current: &Version,
    channel: ReleaseChannel,
    target: PackageTarget,
) -> Result<Option<UpdateCandidate>, UpdateError> {
    if body.len() > MAX_RELEASE_INDEX_BYTES {
        return Err(UpdateError::InvalidResponse);
    }
    let releases: Vec<GithubRelease> =
        serde_json::from_slice(body).map_err(|_| UpdateError::InvalidResponse)?;
    if releases.len() > MAX_RELEASES {
        return Err(UpdateError::InvalidResponse);
    }

    let mut selected: Option<UpdateCandidate> = None;
    for release in releases {
        if release.assets.len() > MAX_RELEASE_ASSETS {
            return Err(UpdateError::InvalidResponse);
        }
        let Some(version) = validated_release_version(&release) else {
            continue;
        };
        if release.draft
            || version <= *current
            || (channel == ReleaseChannel::Stable && release.prerelease)
            || release.prerelease != !version.pre.is_empty()
        {
            continue;
        }
        let release_page = validated_release_page(&release.html_url, &release.tag_name)?;
        let asset = select_asset(&release.assets, &version, target)?;
        let candidate = UpdateCandidate {
            version,
            release_page,
            asset,
        };
        if selected
            .as_ref()
            .is_none_or(|current| candidate.version > current.version)
        {
            selected = Some(candidate);
        }
    }
    Ok(selected)
}

fn validated_release_version(release: &GithubRelease) -> Option<Version> {
    if release.tag_name.is_empty()
        || release.tag_name.len() > MAX_TAG_BYTES
        || release.tag_name.chars().any(char::is_control)
    {
        return None;
    }
    Version::parse(release.tag_name.strip_prefix('v')?).ok()
}

fn validated_release_page(url: &str, tag: &str) -> Result<String, UpdateError> {
    if url.len() > MAX_RELEASE_URL_BYTES
        || url.chars().any(char::is_control)
        || url != format!("{RELEASE_PAGE_PREFIX}{tag}")
    {
        return Err(UpdateError::InvalidResponse);
    }
    let parsed = Url::parse(url).map_err(|_| UpdateError::InvalidResponse)?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(UpdateError::InvalidResponse);
    }
    Ok(url.to_owned())
}

fn select_asset(
    assets: &[GithubAsset],
    version: &Version,
    target: PackageTarget,
) -> Result<Option<UpdateAsset>, UpdateError> {
    let Some(expected_name) = expected_asset_name(version, target) else {
        return Ok(None);
    };
    let mut matching = assets.iter().filter(|asset| asset.name == expected_name);
    let Some(asset) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(UpdateError::InvalidResponse);
    }
    Ok(validate_asset(asset).ok())
}

fn validate_asset(asset: &GithubAsset) -> Result<UpdateAsset, UpdateError> {
    if asset.name.is_empty()
        || asset.name.len() > MAX_ASSET_NAME_BYTES
        || asset.name.chars().any(char::is_control)
        || asset.url.len() > MAX_ASSET_URL_BYTES
        || asset.url.chars().any(char::is_control)
        || asset.size == 0
        || asset.size > MAX_PACKAGE_BYTES
    {
        return Err(UpdateError::InvalidResponse);
    }
    let parsed = Url::parse(&asset.url).map_err(|_| UpdateError::InvalidResponse)?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("api.github.com")
        || !parsed.path().starts_with(ASSET_API_PREFIX)
        || parsed.path()[ASSET_API_PREFIX.len()..].is_empty()
        || !parsed.path()[ASSET_API_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(UpdateError::InvalidResponse);
    }
    let sha256 = parse_sha256(
        asset
            .digest
            .as_deref()
            .ok_or(UpdateError::InvalidResponse)?,
    )?;
    Ok(UpdateAsset {
        name: asset.name.clone(),
        api_url: asset.url.clone(),
        size: asset.size,
        sha256,
    })
}

fn parse_sha256(value: &str) -> Result<[u8; 32], UpdateError> {
    let hex = value
        .strip_prefix(SHA256_PREFIX)
        .filter(|hex| hex.len() == 64)
        .ok_or(UpdateError::InvalidResponse)?;
    let mut digest = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_digit(pair[0]).ok_or(UpdateError::InvalidResponse)?;
        let low = hex_digit(pair[1]).ok_or(UpdateError::InvalidResponse)?;
        digest[index] = high << 4 | low;
    }
    Ok(digest)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn expected_asset_name(version: &Version, target: PackageTarget) -> Option<String> {
    match target {
        PackageTarget::Arch => Some(arch_asset_name(version)),
        PackageTarget::Rpm { fedora_major } | PackageTarget::RpmOstree { fedora_major } => {
            Some(rpm_asset_name(version, fedora_major))
        }
        PackageTarget::Deb => Some(deb_asset_name(version)),
        PackageTarget::Manual => None,
    }
}

pub(crate) fn arch_asset_name(version: &Version) -> String {
    let mut normalized = base_version(version);
    normalized.extend(
        version
            .pre
            .as_str()
            .bytes()
            .filter(u8::is_ascii_alphanumeric)
            .map(char::from),
    );
    format!("overcrow-bin-{normalized}-1-x86_64.pkg.tar.zst")
}

pub(crate) fn rpm_asset_name(version: &Version, fedora_major: u16) -> String {
    let mut normalized = base_version(version);
    if !version.pre.is_empty() {
        normalized.push('.');
        normalized.push_str(&version.pre.as_str().replace('-', "_"));
    }
    format!("overcrow-{normalized}-1.fc{fedora_major}.x86_64.rpm")
}

pub(crate) fn deb_asset_name(version: &Version) -> String {
    let mut normalized = base_version(version);
    if !version.pre.is_empty() {
        normalized.push('~');
        normalized.push_str(&version.pre.as_str().replace('-', "."));
    }
    format!("overcrow_{normalized}-1_amd64.deb")
}

fn base_version(version: &Version) -> String {
    format!("{}.{}.{}", version.major, version.minor, version.patch)
}
