use std::{
    collections::BTreeSet,
    env,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const MAX_ENVIRONMENT_LABEL_BYTES: usize = 96;
pub(crate) const MAX_PCI_DEVICES: usize = 256;
const MAX_ENVIRONMENT_SOURCE_BYTES: usize = 128;
const MAX_OS_RELEASE_BYTES: usize = 16 * 1024;
const MAX_PCI_METADATA_BYTES: u64 = 16;
const PCI_DEVICES_PATH: &str = "/sys/bus/pci/devices";
const OS_RELEASE_PATHS: [&str; 3] = [
    "/run/host/os-release",
    "/etc/os-release",
    "/usr/lib/os-release",
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EnvironmentIdentity {
    pub session_type: Option<String>,
    pub current_desktop: Option<String>,
    pub desktop_session: Option<String>,
    pub os_name: Option<String>,
}

impl EnvironmentIdentity {
    pub fn from_current_process() -> Self {
        let os_release = OS_RELEASE_PATHS
            .iter()
            .find_map(|path| read_bounded_os_release(Path::new(path)));
        environment_identity_from_sources(|name| env::var_os(name), os_release.as_deref())
    }
}

pub(crate) fn environment_identity_from_sources(
    mut variable: impl FnMut(&str) -> Option<OsString>,
    os_release: Option<&[u8]>,
) -> EnvironmentIdentity {
    EnvironmentIdentity {
        session_type: bounded_environment_value(variable("XDG_SESSION_TYPE")),
        current_desktop: bounded_environment_value(variable("XDG_CURRENT_DESKTOP")),
        desktop_session: bounded_environment_value(variable("DESKTOP_SESSION")),
        os_name: os_release.and_then(parse_os_release_name),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplaySession {
    Wayland,
    X11,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopEnvironment {
    Hyprland,
    Plasma,
    Gnome,
    Sway,
    Xfce,
    Gamescope,
    Other,
    Ambiguous,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Supported,
    ValidationInProgress,
    ExperimentalForNow,
    NotCompatibleForNow,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityReason {
    HyprlandWayland,
    PlasmaWayland,
    GenericX11,
    GnomeWayland,
    SwayWayland,
    GamescopeSession,
    OtherWayland,
    AmbiguousDesktop,
    UnknownSession,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsVendor {
    Amd,
    Intel,
    Nvidia,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityReport {
    pub operating_system: String,
    pub session: DisplaySession,
    pub desktop: DesktopEnvironment,
    pub status: CompatibilityStatus,
    pub reason: CompatibilityReason,
    pub activation_allowed: bool,
    pub graphics: Vec<GraphicsVendor>,
}

impl CompatibilityReport {
    pub fn from_environment(identity: EnvironmentIdentity) -> Self {
        let session = classify_session(identity.session_type.as_deref());
        let desktop = classify_desktop(
            identity.current_desktop.as_deref(),
            identity.desktop_session.as_deref(),
        );
        let (status, reason, activation_allowed) = classify_compatibility(session, desktop);

        Self {
            operating_system: bounded_label(identity.os_name.as_deref(), "Unknown Linux"),
            session,
            desktop,
            status,
            reason,
            activation_allowed,
            graphics: Vec::new(),
        }
    }

    pub fn with_graphics(mut self, graphics: Vec<GraphicsVendor>) -> Self {
        self.graphics = normalize_graphics_vendors(graphics);
        self
    }
}

pub fn detect_current_graphics_vendors() -> Vec<GraphicsVendor> {
    detect_graphics_vendors(Path::new(PCI_DEVICES_PATH))
}

pub fn detect_graphics_vendors(root: &Path) -> Vec<GraphicsVendor> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    detect_graphics_vendors_from_paths(
        entries.filter_map(|entry| entry.ok().map(|entry| entry.path())),
    )
}

pub(crate) fn detect_graphics_vendors_from_paths(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<GraphicsVendor> {
    let vendors = paths
        .into_iter()
        .take(MAX_PCI_DEVICES)
        .filter_map(|device| graphics_vendor_from_device(&device));
    normalize_graphics_vendors(vendors)
}

fn graphics_vendor_from_device(device: &Path) -> Option<GraphicsVendor> {
    let class = read_bounded_pci_hex(&device.join("class"))?;
    if class >> 16 != 0x03 {
        return None;
    }

    Some(match read_bounded_pci_hex(&device.join("vendor"))? {
        0x1002 => GraphicsVendor::Amd,
        0x8086 => GraphicsVendor::Intel,
        0x10de => GraphicsVendor::Nvidia,
        _ => GraphicsVendor::Other,
    })
}

fn read_bounded_pci_hex(path: &Path) -> Option<u32> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }

    let mut bytes = Vec::with_capacity(MAX_PCI_METADATA_BYTES as usize);
    File::open(path)
        .ok()?
        .take(MAX_PCI_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PCI_METADATA_BYTES {
        return None;
    }

    let value = std::str::from_utf8(&bytes).ok()?.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(value, 16).ok()
}

fn normalize_graphics_vendors(
    graphics: impl IntoIterator<Item = GraphicsVendor>,
) -> Vec<GraphicsVendor> {
    graphics
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn classify_session(value: Option<&str>) -> DisplaySession {
    match value.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("wayland") => DisplaySession::Wayland,
        Some(value) if value.eq_ignore_ascii_case("x11") => DisplaySession::X11,
        _ => DisplaySession::Unknown,
    }
}

fn classify_desktop(
    current_desktop: Option<&str>,
    desktop_session: Option<&str>,
) -> DesktopEnvironment {
    let recognized = current_desktop
        .into_iter()
        .chain(desktop_session)
        .flat_map(|value| value.split(':'))
        .filter_map(classify_desktop_token)
        .collect::<BTreeSet<_>>();

    match recognized.len() {
        0 if has_desktop_metadata(current_desktop, desktop_session) => DesktopEnvironment::Other,
        0 => DesktopEnvironment::Unknown,
        1 => recognized
            .into_iter()
            .next()
            .unwrap_or(DesktopEnvironment::Unknown),
        _ => DesktopEnvironment::Ambiguous,
    }
}

fn has_desktop_metadata(current_desktop: Option<&str>, desktop_session: Option<&str>) -> bool {
    [current_desktop, desktop_session]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
}

fn classify_desktop_token(token: &str) -> Option<DesktopEnvironment> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("hyprland") {
        return Some(DesktopEnvironment::Hyprland);
    }
    if [
        "kde",
        "plasma",
        "plasmawayland",
        "plasma-wayland",
        "plasma6",
        "kde-plasma",
        "kde-plasma-wayland",
    ]
    .iter()
    .any(|known| token.eq_ignore_ascii_case(known))
    {
        return Some(DesktopEnvironment::Plasma);
    }
    if [
        "gnome",
        "gnome-wayland",
        "ubuntu",
        "ubuntu-gnome",
        "ubuntu-wayland",
    ]
    .iter()
    .any(|known| token.eq_ignore_ascii_case(known))
    {
        return Some(DesktopEnvironment::Gnome);
    }
    if ["sway", "swayfx"]
        .iter()
        .any(|known| token.eq_ignore_ascii_case(known))
    {
        return Some(DesktopEnvironment::Sway);
    }
    if ["xfce", "xfce4", "xubuntu"]
        .iter()
        .any(|known| token.eq_ignore_ascii_case(known))
    {
        return Some(DesktopEnvironment::Xfce);
    }
    token
        .eq_ignore_ascii_case("gamescope")
        .then_some(DesktopEnvironment::Gamescope)
}

fn classify_compatibility(
    session: DisplaySession,
    desktop: DesktopEnvironment,
) -> (CompatibilityStatus, CompatibilityReason, bool) {
    match (session, desktop) {
        (_, DesktopEnvironment::Ambiguous) => (
            CompatibilityStatus::Unknown,
            CompatibilityReason::AmbiguousDesktop,
            false,
        ),
        (DisplaySession::Wayland, DesktopEnvironment::Hyprland) => (
            CompatibilityStatus::Supported,
            CompatibilityReason::HyprlandWayland,
            true,
        ),
        (DisplaySession::Wayland, DesktopEnvironment::Plasma) => (
            CompatibilityStatus::ValidationInProgress,
            CompatibilityReason::PlasmaWayland,
            true,
        ),
        (DisplaySession::Wayland, DesktopEnvironment::Gnome) => (
            CompatibilityStatus::ExperimentalForNow,
            CompatibilityReason::GnomeWayland,
            true,
        ),
        (DisplaySession::Wayland, DesktopEnvironment::Sway) => (
            CompatibilityStatus::NotCompatibleForNow,
            CompatibilityReason::SwayWayland,
            false,
        ),
        (_, DesktopEnvironment::Gamescope) => (
            CompatibilityStatus::NotCompatibleForNow,
            CompatibilityReason::GamescopeSession,
            false,
        ),
        (DisplaySession::X11, _) => (
            CompatibilityStatus::ExperimentalForNow,
            CompatibilityReason::GenericX11,
            true,
        ),
        (DisplaySession::Wayland, _) => (
            CompatibilityStatus::NotCompatibleForNow,
            CompatibilityReason::OtherWayland,
            false,
        ),
        (DisplaySession::Unknown, _) => (
            CompatibilityStatus::Unknown,
            CompatibilityReason::UnknownSession,
            false,
        ),
    }
}

fn bounded_label(value: Option<&str>, fallback: &str) -> String {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    let mut label = value.unwrap_or(fallback).to_owned();
    if label.len() <= MAX_ENVIRONMENT_LABEL_BYTES {
        return label;
    }

    let mut end = MAX_ENVIRONMENT_LABEL_BYTES;
    while end > 0 && !label.is_char_boundary(end) {
        end -= 1;
    }
    label.truncate(end);
    label
}

fn bounded_environment_value(value: Option<OsString>) -> Option<String> {
    let value = value?.into_string().ok()?;
    (value.len() <= MAX_ENVIRONMENT_SOURCE_BYTES).then_some(value)
}

fn read_bounded_os_release(path: &Path) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_OS_RELEASE_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_OS_RELEASE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= MAX_OS_RELEASE_BYTES).then_some(bytes)
}

fn parse_os_release_name(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_OS_RELEASE_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?;
    parse_os_release_value(value)
}

fn parse_os_release_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let (content, quoted) = match (value.as_bytes().first(), value.as_bytes().last()) {
        (Some(b'"'), Some(b'"')) if value.len() >= 2 => (&value[1..value.len() - 1], true),
        (Some(b'\''), Some(b'\'')) if value.len() >= 2 => (&value[1..value.len() - 1], true),
        (Some(b'"' | b'\''), _) | (_, Some(b'"' | b'\'')) => return None,
        _ if value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte == b'\\') =>
        {
            return None;
        }
        _ => (value, false),
    };

    if !quoted {
        return Some(bounded_label(Some(content), "Unknown Linux"));
    }

    let mut decoded = String::with_capacity(content.len().min(MAX_ENVIRONMENT_LABEL_BYTES));
    let mut characters = content.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            decoded.push(characters.next()?);
        } else {
            decoded.push(character);
        }
        if decoded.len() > MAX_ENVIRONMENT_LABEL_BYTES * 2 {
            break;
        }
    }
    Some(bounded_label(Some(&decoded), "Unknown Linux"))
}
