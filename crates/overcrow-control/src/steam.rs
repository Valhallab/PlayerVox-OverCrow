use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

use keyvalues_parser::{Obj, Value};

use crate::presentation::{
    MAX_CONTROL_GAME_NAME_BYTES, MAX_CONTROL_MESSAGE_BYTES, bounded_control_text,
};

/// Maximum accepted size of `libraryfolders.vdf` (4 MiB).
pub const MAX_LIBRARY_VDF_BYTES: usize = 4 * 1024 * 1024;
/// Maximum accepted size of one app manifest (1 MiB).
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
/// Maximum number of unique secondary libraries accepted per discovery run.
pub const MAX_SECONDARY_LIBRARIES: usize = 64;
/// Maximum number of manifest-like directory entries parsed per discovery run.
pub const MAX_MANIFESTS_INSPECTED: usize = 256;
/// Maximum number of warning strings retained per discovery run.
pub const MAX_WARNINGS: usize = 64;
/// Maximum object nesting accepted before invoking the KeyValues parser.
pub const MAX_KEYVALUES_NESTING_DEPTH: usize = 64;
const MAX_PATH_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SteamGame {
    pub app_id: u32,
    pub name: String,
    pub install_dir: PathBuf,
    pub icon: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryReport {
    pub games: Vec<SteamGame>,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
struct SteamLibrary {
    root: PathBuf,
    steamapps: PathBuf,
    icon_root: PathBuf,
}

struct LocatedError {
    path: PathBuf,
    message: String,
}

pub fn candidate_steam_roots(home: &Path) -> Vec<PathBuf> {
    let candidates = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    let mut seen = BTreeSet::new();

    candidates
        .into_iter()
        .filter_map(|candidate| candidate.canonicalize().ok())
        .filter(|candidate| candidate.is_dir())
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

pub fn discover_from_roots(roots: &[PathBuf]) -> DiscoveryReport {
    let mut report = DiscoveryReport::default();
    let mut games = BTreeMap::new();
    let mut seen_libraries = BTreeSet::new();
    let mut libraries = Vec::new();
    let mut secondary_library_count = 0;
    let mut manifests_inspected = 0;
    let mut manifest_limit_reported = false;

    for requested_root in roots {
        let root = match canonical_directory(requested_root) {
            Ok(root) => root,
            Err(error) => {
                push_warning(&mut report.warnings, path_warning(requested_root, &error));
                continue;
            }
        };

        let root_library = match canonical_library(&root, &root) {
            Ok(library) => library,
            Err(error) => {
                push_warning(
                    &mut report.warnings,
                    path_warning(&error.path, &error.message),
                );
                continue;
            }
        };
        if seen_libraries.insert(root_library.root.clone()) {
            libraries.push(root_library.clone());
        }

        let library_file = root_library.steamapps.join("libraryfolders.vdf");
        let canonical_library_file = match canonical_regular_file_within(
            &root_library.steamapps,
            &library_file,
            "canonical steamapps",
        ) {
            Ok(path) => path,
            Err(error) => {
                push_warning(
                    &mut report.warnings,
                    path_warning(&error.path, &error.message),
                );
                continue;
            }
        };
        let text = match read_bounded_keyvalues(&canonical_library_file, MAX_LIBRARY_VDF_BYTES) {
            Ok(text) => text,
            Err(error) => {
                push_warning(
                    &mut report.warnings,
                    path_warning(
                        &library_file,
                        &format!("could not read Steam library list: {error}"),
                    ),
                );
                continue;
            }
        };

        match library_paths(&text) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(path) => match canonical_directory(&path) {
                            Ok(path) => {
                                if seen_libraries.contains(&path) {
                                    continue;
                                }
                                if secondary_library_count == MAX_SECONDARY_LIBRARIES {
                                    push_warning(
                                        &mut report.warnings,
                                        path_warning(
                                            &library_file,
                                            &format!(
                                                "secondary library limit of {MAX_SECONDARY_LIBRARIES} reached"
                                            ),
                                        ),
                                    );
                                    break;
                                }
                                match canonical_library(&path, &root) {
                                    Ok(library) => {
                                        if seen_libraries.insert(library.root.clone()) {
                                            libraries.push(library);
                                            secondary_library_count += 1;
                                        }
                                    }
                                    Err(error) => push_warning(
                                        &mut report.warnings,
                                        path_warning(&error.path, &error.message),
                                    ),
                                }
                            }
                            Err(error) => push_warning(
                                &mut report.warnings,
                                path_warning(&path, &format!("invalid Steam library: {error}")),
                            ),
                        },
                        Err(error) => {
                            push_warning(&mut report.warnings, path_warning(&library_file, &error))
                        }
                    }
                }
            }
            Err(error) => push_warning(
                &mut report.warnings,
                path_warning(
                    &library_file,
                    &format!("invalid Steam library list: {error}"),
                ),
            ),
        }
    }

    for library in libraries {
        if manifest_limit_reported {
            break;
        }
        scan_library(
            &library,
            &mut games,
            &mut report.warnings,
            &mut manifests_inspected,
            &mut manifest_limit_reported,
        );
    }

    report.games = games.into_values().collect();
    report
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("could not canonicalize directory: {error}"))?;
    if !canonical.is_dir() {
        return Err("path is not a directory".to_owned());
    }
    Ok(canonical)
}

fn canonical_library(root: &Path, icon_root: &Path) -> Result<SteamLibrary, LocatedError> {
    let root = canonical_directory(root).map_err(|message| LocatedError {
        path: root.to_owned(),
        message,
    })?;
    let icon_root = canonical_directory(icon_root).map_err(|message| LocatedError {
        path: icon_root.to_owned(),
        message,
    })?;
    let requested_steamapps = root.join("steamapps");
    let steamapps = canonical_directory(&requested_steamapps).map_err(|message| LocatedError {
        path: requested_steamapps.clone(),
        message,
    })?;
    if !is_strictly_contained(&root, &steamapps) {
        return Err(LocatedError {
            path: requested_steamapps,
            message: "steamapps resolves outside canonical library root".to_owned(),
        });
    }

    Ok(SteamLibrary {
        root,
        steamapps,
        icon_root,
    })
}

fn canonical_regular_file_within(
    base: &Path,
    requested: &Path,
    base_name: &str,
) -> Result<PathBuf, LocatedError> {
    let canonical = requested.canonicalize().map_err(|error| LocatedError {
        path: requested.to_owned(),
        message: format!("could not canonicalize file: {error}"),
    })?;
    if !canonical.is_file() {
        return Err(LocatedError {
            path: requested.to_owned(),
            message: "path is not a regular file".to_owned(),
        });
    }
    if !is_strictly_contained(base, &canonical) {
        return Err(LocatedError {
            path: requested.to_owned(),
            message: format!("file resolves outside {base_name}"),
        });
    }
    Ok(canonical)
}

fn canonical_directory_within(
    base: &Path,
    requested: &Path,
    base_name: &str,
) -> Result<PathBuf, LocatedError> {
    let canonical = canonical_directory(requested).map_err(|message| LocatedError {
        path: requested.to_owned(),
        message,
    })?;
    if !is_strictly_contained(base, &canonical) {
        return Err(LocatedError {
            path: requested.to_owned(),
            message: format!("directory resolves outside {base_name}"),
        });
    }
    Ok(canonical)
}

fn is_strictly_contained(base: &Path, candidate: &Path) -> bool {
    candidate != base && candidate.starts_with(base)
}

fn library_paths(text: &str) -> Result<Vec<Result<PathBuf, String>>, String> {
    let document = keyvalues_parser::parse(text).map_err(|error| error.to_string())?;
    if !document.key.eq_ignore_ascii_case("libraryfolders") {
        return Err("expected the libraryfolders root object".to_owned());
    }
    let object = document
        .value
        .get_obj()
        .ok_or_else(|| "libraryfolders must be an object".to_owned())?;
    let mut numbered_entries = object
        .iter()
        .filter_map(|(key, values)| key.parse::<u32>().ok().map(|index| (index, values)))
        .collect::<Vec<_>>();
    numbered_entries.sort_by_key(|(index, _)| *index);

    Ok(numbered_entries
        .into_iter()
        .map(|(index, values)| library_path(index, values))
        .collect())
}

fn library_path(index: u32, values: &[Value<'_>]) -> Result<PathBuf, String> {
    let value = only_value(values)
        .ok_or_else(|| format!("library entry {index} must occur exactly once"))?;
    let path = if let Some(path) = value.get_str() {
        path
    } else {
        let object = value
            .get_obj()
            .ok_or_else(|| format!("library entry {index} must be a path string or an object"))?;
        object_string(object, "path").map_err(|error| format!("library entry {index}: {error}"))?
    };
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("library entry {index}: path must be absolute"));
    }
    Ok(path)
}

fn scan_library(
    library: &SteamLibrary,
    games: &mut BTreeMap<u32, SteamGame>,
    warnings: &mut Vec<String>,
    manifests_inspected: &mut usize,
    manifest_limit_reported: &mut bool,
) {
    let steamapps = &library.steamapps;
    let entries = match fs::read_dir(steamapps) {
        Ok(entries) => entries,
        Err(error) => {
            push_warning(
                warnings,
                path_warning(steamapps, &format!("could not read Steam library: {error}")),
            );
            return;
        }
    };
    let remaining = MAX_MANIFESTS_INSPECTED.saturating_sub(*manifests_inspected);
    let mut valid_manifests = BTreeSet::new();
    let mut malformed_names = BTreeSet::new();
    let mut exceeded_manifest_limit = false;
    let mut exceeded_malformed_warning_limit = false;
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                match manifest_filename_app_id(&path) {
                    ManifestFilename::Valid(app_id) => retain_bounded(
                        &mut valid_manifests,
                        (app_id, path),
                        remaining,
                        &mut exceeded_manifest_limit,
                    ),
                    ManifestFilename::Malformed => retain_bounded(
                        &mut malformed_names,
                        path,
                        MAX_WARNINGS,
                        &mut exceeded_malformed_warning_limit,
                    ),
                    ManifestFilename::Unrelated => {}
                }
            }
            Err(error) => push_warning(
                warnings,
                path_warning(
                    steamapps,
                    &format!("could not inspect Steam library entry: {error}"),
                ),
            ),
        }
    }

    for path in malformed_names {
        push_warning(
            warnings,
            path_warning(
                &path,
                "malformed app manifest filename; expected appmanifest_<appid>.acf",
            ),
        );
    }
    if exceeded_malformed_warning_limit {
        push_warning(
            warnings,
            path_warning(
                steamapps,
                "malformed app manifest filename warning limit reached",
            ),
        );
    }

    *manifests_inspected += valid_manifests.len();
    if exceeded_manifest_limit {
        push_warning(
            warnings,
            path_warning(
                steamapps,
                &format!("manifest inspection limit of {MAX_MANIFESTS_INSPECTED} reached"),
            ),
        );
        *manifest_limit_reported = true;
    }

    for (filename_app_id, manifest) in valid_manifests {
        let canonical_manifest =
            match canonical_regular_file_within(steamapps, &manifest, "canonical steamapps") {
                Ok(path) => path,
                Err(error) => {
                    push_warning(warnings, path_warning(&error.path, &error.message));
                    continue;
                }
            };
        match parse_manifest(
            &canonical_manifest,
            &manifest,
            filename_app_id,
            library,
            warnings,
        ) {
            Ok(Some(game)) => {
                games.entry(game.app_id).or_insert(game);
            }
            Ok(None) => {}
            Err(error) => {
                push_warning(warnings, path_warning(&error.path, &error.message));
            }
        }
    }
}

fn retain_bounded<T: Ord>(
    values: &mut BTreeSet<T>,
    value: T,
    limit: usize,
    exceeded_limit: &mut bool,
) {
    if values.contains(&value) {
        return;
    }
    if values.len() < limit {
        values.insert(value);
        return;
    }

    *exceeded_limit = true;
    if values.last().is_some_and(|last| value < *last) {
        values.pop_last();
        values.insert(value);
    }
}

enum ManifestFilename {
    Valid(u32),
    Malformed,
    Unrelated,
}

fn manifest_filename_app_id(path: &Path) -> ManifestFilename {
    let Some(filename) = path.file_name() else {
        return ManifestFilename::Unrelated;
    };
    let lossy = filename.to_string_lossy();
    if !lossy.starts_with("appmanifest_") || !lossy.ends_with(".acf") {
        return ManifestFilename::Unrelated;
    }
    let Some(filename) = filename.to_str() else {
        return ManifestFilename::Malformed;
    };
    let Some(app_id) = filename
        .strip_prefix("appmanifest_")
        .and_then(|name| name.strip_suffix(".acf"))
        .and_then(|name| name.parse().ok())
    else {
        return ManifestFilename::Malformed;
    };
    ManifestFilename::Valid(app_id)
}

fn parse_manifest(
    read_path: &Path,
    manifest_path: &Path,
    filename_app_id: u32,
    library: &SteamLibrary,
    warnings: &mut Vec<String>,
) -> Result<Option<SteamGame>, LocatedError> {
    let text = read_bounded_keyvalues(read_path, MAX_MANIFEST_BYTES).map_err(|error| {
        manifest_error(
            manifest_path,
            format!("could not read Steam app manifest: {error}"),
        )
    })?;
    let document = keyvalues_parser::parse(&text).map_err(|error| {
        manifest_error(
            manifest_path,
            format!("invalid Steam app manifest: {error}"),
        )
    })?;
    if !document.key.eq_ignore_ascii_case("AppState") {
        return Err(manifest_error(
            manifest_path,
            "invalid Steam app manifest: expected the AppState root object",
        ));
    }
    let object = document.value.get_obj().ok_or_else(|| {
        manifest_error(
            manifest_path,
            "invalid Steam app manifest: AppState must be an object",
        )
    })?;

    let app_id = object_string(object, "appid")
        .map_err(|error| manifest_error(manifest_path, error))?
        .parse::<u32>()
        .map_err(|_| {
            manifest_error(
                manifest_path,
                "manifest appid must be a 32-bit unsigned integer",
            )
        })?;
    if app_id != filename_app_id {
        return Err(manifest_error(
            manifest_path,
            format!("manifest appid {app_id} does not match filename app ID {filename_app_id}"),
        ));
    }
    if app_id == 0 {
        return Ok(None);
    }

    let name =
        object_string(object, "name").map_err(|error| manifest_error(manifest_path, error))?;
    if name.trim().is_empty() {
        return Err(manifest_error(
            manifest_path,
            "manifest name must not be empty",
        ));
    }
    if name.len() > MAX_CONTROL_GAME_NAME_BYTES {
        return Err(manifest_error(
            manifest_path,
            format!("manifest name exceeds byte limit {MAX_CONTROL_GAME_NAME_BYTES}"),
        ));
    }
    let install_directory_name = object_string(object, "installdir")
        .map_err(|error| manifest_error(manifest_path, error))?;
    if !is_safe_component(install_directory_name) {
        return Err(manifest_error(
            manifest_path,
            "manifest installdir must be one safe path component",
        ));
    }
    let Some(install_dir) = canonical_install_directory(library, install_directory_name)? else {
        return Ok(None);
    };
    let icon = match optional_object_string(object, "icon") {
        Some(value) if !is_safe_component(value) => {
            push_warning(
                warnings,
                path_warning(
                    manifest_path,
                    "manifest icon must be one safe path component",
                ),
            );
            None
        }
        Some(value) => match canonical_local_icon(library, value) {
            Ok(icon) => icon,
            Err(error) => {
                push_warning(warnings, path_warning(&error.path, &error.message));
                None
            }
        },
        None => None,
    };

    Ok(Some(SteamGame {
        app_id,
        name: name.to_owned(),
        install_dir,
        icon,
    }))
}

fn canonical_install_directory(
    library: &SteamLibrary,
    install_directory_name: &str,
) -> Result<Option<PathBuf>, LocatedError> {
    let requested_common = library.steamapps.join("common");
    let common =
        canonical_directory_within(&library.steamapps, &requested_common, "canonical steamapps")?;
    let requested_install = common.join(install_directory_name);
    match fs::symlink_metadata(&requested_install) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(LocatedError {
                path: requested_install,
                message: format!("could not inspect install directory: {error}"),
            });
        }
    }
    canonical_directory_within(&common, &requested_install, "canonical common directory").map(Some)
}

fn canonical_local_icon(
    library: &SteamLibrary,
    icon_name: &str,
) -> Result<Option<PathBuf>, LocatedError> {
    let requested_games = library.icon_root.join("steam/games");
    let games = match requested_games.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(LocatedError {
                path: requested_games,
                message: format!("could not canonicalize Steam games directory: {error}"),
            });
        }
    };
    if !games.is_dir() {
        return Err(LocatedError {
            path: requested_games,
            message: "Steam games path is not a directory".to_owned(),
        });
    }
    if !is_strictly_contained(&library.icon_root, &games) {
        return Err(LocatedError {
            path: requested_games,
            message: "Steam games directory resolves outside canonical icon root".to_owned(),
        });
    }

    let requested_icon = games.join(format!("{icon_name}.ico"));
    let canonical_icon = match requested_icon.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(LocatedError {
                path: requested_icon,
                message: format!("could not canonicalize local Steam icon: {error}"),
            });
        }
    };
    if !canonical_icon.is_file() {
        return Err(LocatedError {
            path: requested_icon,
            message: "local Steam icon is not a regular file".to_owned(),
        });
    }
    if !is_strictly_contained(&games, &canonical_icon) {
        return Err(LocatedError {
            path: requested_icon,
            message: "icon resolves outside canonical Steam games directory".to_owned(),
        });
    }
    Ok(Some(canonical_icon))
}

fn manifest_error(path: &Path, message: impl Into<String>) -> LocatedError {
    LocatedError {
        path: path.to_owned(),
        message: message.into(),
    }
}

fn object_string<'object, 'text>(
    object: &'object Obj<'text>,
    key: &str,
) -> Result<&'object str, String> {
    let values = object
        .get(key)
        .ok_or_else(|| format!("required key {key:?} is missing"))?;
    only_value(values)
        .and_then(Value::get_str)
        .ok_or_else(|| format!("key {key:?} must occur exactly once and contain a string"))
}

fn optional_object_string<'object, 'text>(
    object: &'object Obj<'text>,
    key: &str,
) -> Option<&'object str> {
    object
        .get(key)
        .and_then(|values| only_value(values))
        .and_then(Value::get_str)
}

fn only_value<'values, 'text>(values: &'values [Value<'text>]) -> Option<&'values Value<'text>> {
    match values {
        [value] => Some(value),
        _ => None,
    }
}

fn is_safe_component(value: &str) -> bool {
    if value.len() > MAX_PATH_COMPONENT_BYTES {
        return false;
    }
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn path_warning(path: &Path, message: &str) -> String {
    format!("{}: {message}", path.display())
}

fn read_bounded_keyvalues(path: &Path, byte_limit: usize) -> Result<String, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if metadata.len() > byte_limit as u64 {
        return Err(format!(
            "file size {} exceeds byte limit {byte_limit}",
            metadata.len()
        ));
    }

    let mut bytes = Vec::with_capacity(metadata.len().min(byte_limit as u64) as usize);
    file.take(byte_limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > byte_limit {
        return Err(format!(
            "file content exceeds byte limit {byte_limit} while reading"
        ));
    }
    preflight_keyvalues_nesting(&bytes)?;
    String::from_utf8(bytes).map_err(|_| "file is not valid UTF-8".to_owned())
}

fn preflight_keyvalues_nesting(bytes: &[u8]) -> Result<(), String> {
    #[derive(Clone, Copy)]
    enum State {
        Normal,
        String,
        LineComment,
    }

    let mut state = State::Normal;
    let mut depth = 0;
    let mut index = 0;
    let mut in_unquoted_token = false;
    while index < bytes.len() {
        let byte = bytes[index];
        match state {
            State::Normal => match byte {
                b'"' => {
                    in_unquoted_token = false;
                    state = State::String;
                }
                b'/' if !in_unquoted_token && bytes.get(index + 1) == Some(&b'/') => {
                    state = State::LineComment;
                    index += 1;
                }
                b'{' => {
                    in_unquoted_token = false;
                    depth += 1;
                    if depth > MAX_KEYVALUES_NESTING_DEPTH {
                        return Err(format!(
                            "KeyValues nesting depth limit of {MAX_KEYVALUES_NESTING_DEPTH} exceeded"
                        ));
                    }
                }
                b'}' => {
                    in_unquoted_token = false;
                    if depth == 0 {
                        return Err("unbalanced KeyValues braces".to_owned());
                    }
                    depth -= 1;
                }
                b' ' | b'\t' | b'\r' | b'\n' => in_unquoted_token = false,
                _ => in_unquoted_token = true,
            },
            State::String => match byte {
                b'\\' => index += usize::from(index + 1 < bytes.len()),
                b'"' => state = State::Normal,
                _ => {}
            },
            State::LineComment => {
                if matches!(byte, b'\n' | b'\r') {
                    in_unquoted_token = false;
                    state = State::Normal;
                }
            }
        }
        index += 1;
    }

    if depth != 0 {
        return Err("unbalanced KeyValues braces".to_owned());
    }
    Ok(())
}

fn push_warning(warnings: &mut Vec<String>, warning: String) {
    const TRUNCATION_NOTICE: &str =
        "Steam discovery warning limit reached; further warnings omitted";
    let warning = bounded_control_text(&warning, MAX_CONTROL_MESSAGE_BYTES);

    if warnings.len() < MAX_WARNINGS {
        warnings.push(warning);
    } else if warnings
        .last()
        .is_some_and(|last| last != TRUNCATION_NOTICE)
        && let Some(last) = warnings.last_mut()
    {
        *last = TRUNCATION_NOTICE.to_owned();
    }
}
