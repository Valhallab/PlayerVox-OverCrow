use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use crate::presentation::{
    MAX_CONTROL_GAME_NAME_BYTES, MAX_CONTROL_MESSAGE_BYTES, bounded_control_text,
};

pub(crate) const MAX_APPINFO_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_SHORTCUTS_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_STEAM_PROFILES: usize = 32;
pub(crate) const MAX_USERDATA_ENTRIES: usize = 128;
pub(crate) const MAX_BINARY_VDF_DEPTH: usize = 64;
pub(crate) const MAX_BINARY_VDF_ENTRIES: usize = 100_000;
pub(crate) const MAX_APPINFO_RECORDS: usize = 16_384;
pub(crate) const MAX_SHORTCUT_RECORDS: usize = 1_024;
pub(crate) const MAX_STRING_TABLE_ENTRIES: usize = 32_768;

const APPINFO_MAGIC_V40: u32 = 0x0756_4428;
const APPINFO_MAGIC_V41: u32 = 0x0756_4429;
const APPINFO_ENTRY_HEADER_BYTES: usize = 68;
const APPINFO_BYTES_AFTER_SIZE: usize = 60;
const MAX_BINARY_KEY_BYTES: usize = 256;
const MAX_BINARY_VALUE_BYTES: usize = 1024 * 1024;
const MAX_APP_TYPE_BYTES: usize = 64;
const MAX_BINARY_WARNINGS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SteamAppType {
    Game,
    NonGame,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SteamShortcut {
    pub app_id: u32,
    pub name: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct BinaryMetadataReport {
    pub app_types: BTreeMap<u32, SteamAppType>,
    pub shortcuts: Vec<SteamShortcut>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
enum KeyMode<'a> {
    Inline,
    Indexed(&'a [String]),
}

#[derive(Clone, Copy)]
enum Scalar<'a> {
    String(&'a [u8]),
    Int32(u32),
    Other,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        let value = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| "unexpected end of binary VDF".to_owned())?;
        self.offset += 1;
        Ok(value)
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes(
            bytes
                .try_into()
                .map_err(|_| "invalid 32-bit value".to_owned())?,
        ))
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| "binary VDF offset overflow".to_owned())?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| "unexpected end of binary VDF".to_owned())?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_cstring(&mut self, limit: usize) -> Result<&'a [u8], String> {
        let remaining = self
            .bytes
            .get(self.offset..)
            .ok_or_else(|| "invalid binary VDF offset".to_owned())?;
        let length = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "unterminated binary VDF string".to_owned())?;
        if length > limit {
            return Err(format!("binary VDF string exceeds byte limit {limit}"));
        }
        let value = &remaining[..length];
        self.offset = self
            .offset
            .checked_add(length + 1)
            .ok_or_else(|| "binary VDF offset overflow".to_owned())?;
        Ok(value)
    }

    fn skip_wide_string(&mut self) -> Result<(), String> {
        let start = self.offset;
        while self
            .offset
            .checked_add(1)
            .is_some_and(|end| end < self.bytes.len())
        {
            if self.bytes[self.offset] == 0 && self.bytes[self.offset + 1] == 0 {
                self.offset += 2;
                return Ok(());
            }
            self.offset += 2;
            if self.offset.saturating_sub(start) > MAX_BINARY_VALUE_BYTES {
                return Err(format!(
                    "binary VDF wide string exceeds byte limit {MAX_BINARY_VALUE_BYTES}"
                ));
            }
        }
        Err("unterminated binary VDF wide string".to_owned())
    }
}

pub(crate) fn read_binary_metadata(root: &Path) -> BinaryMetadataReport {
    let mut report = BinaryMetadataReport::default();
    let root = match canonical_directory(root) {
        Ok(root) => root,
        Err(error) => {
            push_warning(
                &mut report.warnings,
                &format!("Steam metadata root is unavailable: {error}"),
            );
            return report;
        }
    };

    match read_optional_file(&root, &root.join("appcache/appinfo.vdf"), MAX_APPINFO_BYTES) {
        Ok(Some(bytes)) => match parse_appinfo(&bytes) {
            Ok(app_types) => report.app_types = app_types,
            Err(error) => push_warning(
                &mut report.warnings,
                &format!("Steam application metadata is invalid: {error}"),
            ),
        },
        Ok(None) => {}
        Err(error) => push_warning(
            &mut report.warnings,
            &format!("Steam application metadata is unavailable: {error}"),
        ),
    }

    read_shortcuts(&root, &mut report);
    report
}

fn read_shortcuts(root: &Path, report: &mut BinaryMetadataReport) {
    let requested_userdata = root.join("userdata");
    let userdata = match canonical_optional_directory_within(root, &requested_userdata) {
        Ok(Some(path)) => path,
        Ok(None) => return,
        Err(error) => {
            push_warning(
                &mut report.warnings,
                &format!("Steam shortcut metadata is unavailable: {error}"),
            );
            return;
        }
    };

    let entries = match fs::read_dir(&userdata) {
        Ok(entries) => entries,
        Err(error) => {
            push_warning(
                &mut report.warnings,
                &format!("Steam shortcut metadata is unavailable: {error}"),
            );
            return;
        }
    };
    let mut profiles = Vec::new();
    for (inspected, entry) in entries.enumerate() {
        if inspected == MAX_USERDATA_ENTRIES {
            push_warning(
                &mut report.warnings,
                &format!("Steam userdata entry limit of {MAX_USERDATA_ENTRIES} reached"),
            );
            break;
        }
        let Ok(entry) = entry else {
            push_warning(
                &mut report.warnings,
                "A Steam userdata entry could not be inspected",
            );
            continue;
        };
        let Some(account) = entry.file_name().to_str().and_then(parse_account_id) else {
            continue;
        };
        profiles.push((account, entry.path()));
    }
    profiles.sort_by_key(|(account, _)| *account);
    if profiles.len() > MAX_STEAM_PROFILES {
        profiles.truncate(MAX_STEAM_PROFILES);
        push_warning(
            &mut report.warnings,
            &format!("Steam profile limit of {MAX_STEAM_PROFILES} reached"),
        );
    }

    for (_, profile) in profiles {
        let profile = match canonical_directory_within(&userdata, &profile) {
            Ok(path) => path,
            Err(error) => {
                push_warning(
                    &mut report.warnings,
                    &format!("A Steam profile could not be inspected: {error}"),
                );
                continue;
            }
        };
        let shortcuts = profile.join("config/shortcuts.vdf");
        let bytes = match read_optional_file(root, &shortcuts, MAX_SHORTCUTS_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(error) => {
                push_warning(
                    &mut report.warnings,
                    &format!("Steam shortcut metadata is unavailable: {error}"),
                );
                continue;
            }
        };
        match parse_shortcuts(&bytes) {
            Ok((shortcuts, rejected)) => {
                if rejected != 0 {
                    push_warning(
                        &mut report.warnings,
                        "Steam shortcut metadata contained an invalid shortcut entry",
                    );
                }
                let remaining = MAX_SHORTCUT_RECORDS.saturating_sub(report.shortcuts.len());
                if shortcuts.len() > remaining {
                    push_warning(
                        &mut report.warnings,
                        &format!("Steam shortcut limit of {MAX_SHORTCUT_RECORDS} reached"),
                    );
                }
                report
                    .shortcuts
                    .extend(shortcuts.into_iter().take(remaining));
            }
            Err(error) => push_warning(
                &mut report.warnings,
                &format!("Steam shortcut metadata is invalid: {error}"),
            ),
        }
    }
}

fn parse_appinfo(bytes: &[u8]) -> Result<BTreeMap<u32, SteamAppType>, String> {
    let mut header = Cursor::new(bytes);
    let magic = header.read_u32()?;
    let _universe = header.read_u32()?;
    let (apps_start, apps_end, string_table) = match magic {
        APPINFO_MAGIC_V40 => (8, bytes.len(), None),
        APPINFO_MAGIC_V41 => {
            let offset_bytes = header.read_exact(8)?;
            let offset = usize::try_from(u64::from_le_bytes(
                offset_bytes
                    .try_into()
                    .map_err(|_| "invalid app-info string-table offset".to_owned())?,
            ))
            .map_err(|_| "app-info string-table offset is too large".to_owned())?;
            if !(16..bytes.len()).contains(&offset) {
                return Err("app-info string-table offset is outside the file".to_owned());
            }
            (16, offset, Some(parse_string_table(&bytes[offset..])?))
        }
        _ => return Err("unsupported app-info format".to_owned()),
    };
    let key_mode = string_table
        .as_deref()
        .map_or(KeyMode::Inline, KeyMode::Indexed);
    let mut offset = apps_start;
    let mut app_types = BTreeMap::new();
    let mut records = 0;

    while offset < apps_end {
        let app_id = read_u32_at(bytes, offset)?;
        if app_id == 0 {
            break;
        }
        if records == MAX_APPINFO_RECORDS {
            return Err(format!(
                "app-info record limit of {MAX_APPINFO_RECORDS} reached"
            ));
        }
        records += 1;
        let size_offset = offset
            .checked_add(4)
            .ok_or_else(|| "app-info size offset overflow".to_owned())?;
        let size = usize::try_from(read_u32_at(bytes, size_offset)?)
            .map_err(|_| "app-info record size is too large".to_owned())?;
        if size < APPINFO_BYTES_AFTER_SIZE {
            return Err("app-info record size is smaller than its header".to_owned());
        }
        let record_bytes = 8_usize
            .checked_add(size)
            .ok_or_else(|| "app-info record size overflow".to_owned())?;
        let end = offset
            .checked_add(record_bytes)
            .ok_or_else(|| "app-info record offset overflow".to_owned())?;
        if end > apps_end {
            return Err("app-info record extends outside the application section".to_owned());
        }
        let vdf_start = offset
            .checked_add(APPINFO_ENTRY_HEADER_BYTES)
            .ok_or_else(|| "app-info VDF offset overflow".to_owned())?;
        let app_type = extract_app_type(&bytes[vdf_start..end], key_mode)?;
        if app_types.insert(app_id, app_type).is_some() {
            return Err("app-info contains a duplicate app ID".to_owned());
        }
        offset = end;
    }
    Ok(app_types)
}

fn parse_string_table(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut cursor = Cursor::new(bytes);
    let count = usize::try_from(cursor.read_u32()?)
        .map_err(|_| "app-info string-table count is too large".to_owned())?;
    if count > MAX_STRING_TABLE_ENTRIES {
        return Err(format!(
            "app-info string-table limit of {MAX_STRING_TABLE_ENTRIES} reached"
        ));
    }
    let mut strings = Vec::with_capacity(count);
    for _ in 0..count {
        let raw = cursor.read_cstring(MAX_BINARY_KEY_BYTES)?;
        let value = std::str::from_utf8(raw)
            .map_err(|_| "app-info string-table key is not valid UTF-8".to_owned())?;
        strings.push(value.to_owned());
    }
    Ok(strings)
}

fn extract_app_type(bytes: &[u8], key_mode: KeyMode<'_>) -> Result<SteamAppType, String> {
    let mut found = None;
    walk_binary_vdf(bytes, key_mode, |path, key, scalar| {
        if path.len() == 2
            && path[0].eq_ignore_ascii_case("appinfo")
            && path[1].eq_ignore_ascii_case("common")
            && key.eq_ignore_ascii_case("type")
        {
            let Scalar::String(raw) = scalar else {
                return Err("app-info type has the wrong value type".to_owned());
            };
            if raw.len() > MAX_APP_TYPE_BYTES {
                return Err(format!(
                    "app-info type exceeds byte limit {MAX_APP_TYPE_BYTES}"
                ));
            }
            let value = std::str::from_utf8(raw)
                .map_err(|_| "app-info type is not valid UTF-8".to_owned())?;
            let app_type = classify_app_type(value);
            if found.replace(app_type).is_some() {
                return Err("app-info contains duplicate type metadata".to_owned());
            }
        }
        Ok(())
    })?;
    Ok(found.unwrap_or(SteamAppType::Unknown))
}

fn classify_app_type(value: &str) -> SteamAppType {
    match value.trim().to_ascii_lowercase().as_str() {
        "game" | "demo" => SteamAppType::Game,
        "advertising" | "application" | "config" | "dlc" | "guide" | "hardware" | "music"
        | "series" | "tool" | "video" => SteamAppType::NonGame,
        _ => SteamAppType::Unknown,
    }
}

fn parse_shortcuts(bytes: &[u8]) -> Result<(Vec<SteamShortcut>, usize), String> {
    #[derive(Default)]
    struct Builder {
        app_id: Option<u32>,
        name: Option<String>,
        invalid: bool,
    }

    let mut builders = BTreeMap::<u32, Builder>::new();
    walk_binary_vdf(bytes, KeyMode::Inline, |path, key, scalar| {
        if path.len() != 2 || !path[0].eq_ignore_ascii_case("shortcuts") {
            return Ok(());
        }
        let Ok(index) = path[1].parse::<u32>() else {
            return Ok(());
        };
        if !builders.contains_key(&index) && builders.len() == MAX_SHORTCUT_RECORDS {
            return Err(format!(
                "Steam shortcut limit of {MAX_SHORTCUT_RECORDS} reached"
            ));
        }
        let builder = builders.entry(index).or_default();
        if key.eq_ignore_ascii_case("appid") {
            let Scalar::Int32(app_id) = scalar else {
                builder.invalid = true;
                return Ok(());
            };
            if builder.app_id.replace(app_id).is_some() {
                builder.invalid = true;
            }
        } else if key.eq_ignore_ascii_case("appname") {
            let Scalar::String(raw) = scalar else {
                builder.invalid = true;
                return Ok(());
            };
            if raw.len() > MAX_CONTROL_GAME_NAME_BYTES {
                builder.invalid = true;
                return Ok(());
            }
            let Ok(name) = std::str::from_utf8(raw) else {
                builder.invalid = true;
                return Ok(());
            };
            if builder.name.replace(name.trim().to_owned()).is_some() {
                builder.invalid = true;
            }
        }
        Ok(())
    })?;

    let mut rejected = 0;
    let shortcuts = builders
        .into_values()
        .filter_map(|builder| {
            let (Some(app_id), Some(name)) = (builder.app_id, builder.name) else {
                rejected += 1;
                return None;
            };
            if builder.invalid || app_id == 0 || name.is_empty() {
                rejected += 1;
                return None;
            }
            Some(SteamShortcut { app_id, name })
        })
        .collect();
    Ok((shortcuts, rejected))
}

fn walk_binary_vdf<'a, F>(
    bytes: &'a [u8],
    key_mode: KeyMode<'_>,
    mut visit: F,
) -> Result<(), String>
where
    F: FnMut(&[String], &str, Scalar<'a>) -> Result<(), String>,
{
    let mut cursor = Cursor::new(bytes);
    let mut path = Vec::new();
    let mut entries = 0;

    while !cursor.is_empty() {
        if entries == MAX_BINARY_VDF_ENTRIES {
            return Err(format!(
                "binary VDF entry limit of {MAX_BINARY_VDF_ENTRIES} reached"
            ));
        }
        entries += 1;
        match cursor.read_u8()? {
            0 => {
                let key = read_key(&mut cursor, key_mode)?;
                if path.len() == MAX_BINARY_VDF_DEPTH {
                    return Err(format!(
                        "binary VDF depth limit of {MAX_BINARY_VDF_DEPTH} reached"
                    ));
                }
                path.push(key);
            }
            1 => {
                let key = read_key(&mut cursor, key_mode)?;
                let value = cursor.read_cstring(MAX_BINARY_VALUE_BYTES)?;
                visit(&path, &key, Scalar::String(value))?;
            }
            2 => {
                let key = read_key(&mut cursor, key_mode)?;
                let value = cursor.read_u32()?;
                visit(&path, &key, Scalar::Int32(value))?;
            }
            3 | 4 | 6 => {
                let key = read_key(&mut cursor, key_mode)?;
                cursor.read_exact(4)?;
                visit(&path, &key, Scalar::Other)?;
            }
            5 => {
                let key = read_key(&mut cursor, key_mode)?;
                cursor.skip_wide_string()?;
                visit(&path, &key, Scalar::Other)?;
            }
            7 => {
                let key = read_key(&mut cursor, key_mode)?;
                cursor.read_exact(8)?;
                visit(&path, &key, Scalar::Other)?;
            }
            8 => {
                if path.pop().is_none() {
                    if cursor.is_empty() {
                        return Ok(());
                    }
                    return Err("binary VDF has an unexpected object end".to_owned());
                }
            }
            value_type => {
                return Err(format!(
                    "binary VDF contains unknown type {value_type:#04x}"
                ));
            }
        }
    }
    if !path.is_empty() {
        return Err("binary VDF contains an unterminated object".to_owned());
    }
    Ok(())
}

fn read_key(cursor: &mut Cursor<'_>, key_mode: KeyMode<'_>) -> Result<String, String> {
    match key_mode {
        KeyMode::Inline => {
            let raw = cursor.read_cstring(MAX_BINARY_KEY_BYTES)?;
            let key = std::str::from_utf8(raw)
                .map_err(|_| "binary VDF key is not valid UTF-8".to_owned())?;
            Ok(key.to_owned())
        }
        KeyMode::Indexed(strings) => {
            let index = usize::try_from(cursor.read_u32()?)
                .map_err(|_| "binary VDF key index is too large".to_owned())?;
            strings
                .get(index)
                .cloned()
                .ok_or_else(|| "binary VDF key index is outside the string table".to_owned())
        }
    }
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "binary metadata offset overflow".to_owned())?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| "unexpected end of binary metadata".to_owned())?;
    Ok(u32::from_le_bytes(
        value
            .try_into()
            .map_err(|_| "invalid 32-bit metadata value".to_owned())?,
    ))
}

fn parse_account_id(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("could not canonicalize directory: {error}"))?;
    canonical
        .is_dir()
        .then_some(canonical)
        .ok_or_else(|| "path is not a directory".to_owned())
}

fn canonical_optional_directory_within(
    root: &Path,
    path: &Path,
) -> Result<Option<PathBuf>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => canonical_directory_within(root, path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not inspect directory: {error}")),
    }
}

fn canonical_directory_within(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let canonical = canonical_directory(path)?;
    is_strictly_contained(root, &canonical)
        .then_some(canonical)
        .ok_or_else(|| "directory resolves outside the Steam root".to_owned())
}

fn read_optional_file(root: &Path, path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not inspect file: {error}")),
    };
    if !metadata.file_type().is_file() {
        return Err("metadata path is not a regular file".to_owned());
    }
    if metadata.len() > u64::try_from(limit).unwrap_or(u64::MAX) {
        return Err(format!("metadata file exceeds byte limit {limit}"));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("could not canonicalize file: {error}"))?;
    if !is_strictly_contained(root, &canonical) {
        return Err("metadata file resolves outside the Steam root".to_owned());
    }

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&canonical)
        .map_err(|error| format!("could not open file: {error}"))?;
    let open_metadata = file
        .metadata()
        .map_err(|error| format!("could not inspect open file: {error}"))?;
    if !open_metadata.is_file() {
        return Err("opened metadata path is not a regular file".to_owned());
    }
    if open_metadata.len() > u64::try_from(limit).unwrap_or(u64::MAX) {
        return Err(format!("metadata file exceeds byte limit {limit}"));
    }
    let take_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let capacity = usize::try_from(open_metadata.len().min(take_limit)).unwrap_or(limit);
    let mut bytes = Vec::with_capacity(capacity);
    file.take(take_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read file: {error}"))?;
    if bytes.len() > limit {
        return Err(format!("metadata file exceeds byte limit {limit}"));
    }
    Ok(Some(bytes))
}

fn is_strictly_contained(root: &Path, path: &Path) -> bool {
    path != root && path.starts_with(root)
}

fn push_warning(warnings: &mut Vec<String>, warning: &str) {
    if warnings.len() >= MAX_BINARY_WARNINGS {
        return;
    }
    warnings.push(bounded_control_text(warning, MAX_CONTROL_MESSAGE_BYTES));
}
