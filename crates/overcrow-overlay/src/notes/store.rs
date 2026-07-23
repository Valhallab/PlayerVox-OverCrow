use std::{
    env,
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;

use super::{NotesDocument, NotesError};

pub const NOTES_FILE_MAX_BYTES: usize = 64 * 1024;
const NOTES_OPEN_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK;

/// Synchronous repository seam for providers admitted to the renderer.
///
/// Implementations run on the owned notes worker and must use bounded local I/O.
/// A future network provider needs an async, cancellable adapter before it can
/// implement this seam without weakening owned shutdown.
pub trait NotesRepository: Send + 'static {
    fn load(&self) -> Result<NotesDocument, NotesError>;
    fn save(&self, document: &NotesDocument) -> Result<(), NotesError>;
}

pub struct LocalNotesRepository {
    path: PathBuf,
}

impl LocalNotesRepository {
    pub fn from_environment() -> Self {
        Self::from_path(notes_path(
            env::var_os("XDG_DATA_HOME").as_deref(),
            env::var_os("HOME").as_deref(),
        ))
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(super) fn save_with_writer<W>(
        &self,
        document: &NotesDocument,
        writer: &W,
    ) -> Result<(), NotesError>
    where
        W: AtomicWriter,
    {
        document.validate()?;
        let mut contents = serde_json::to_vec_pretty(document)?;
        contents.push(b'\n');
        if contents.len() > NOTES_FILE_MAX_BYTES {
            return Err(NotesError::validation(format!(
                "serialized notes exceed {NOTES_FILE_MAX_BYTES} bytes"
            )));
        }

        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| NotesError::repository("user data directory unavailable"))?;
        fs::create_dir_all(parent)?;
        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        writer.write(&mut temporary, &contents)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        writer.persist(temporary, &self.path)?;
        writer.sync_parent(parent).map_err(NotesError::committed)
    }
}

impl NotesRepository for LocalNotesRepository {
    fn load(&self) -> Result<NotesDocument, NotesError> {
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(NOTES_OPEN_FLAGS)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(NotesDocument::default());
            }
            Err(error) => {
                return Err(NotesError::repository(format!(
                    "refusing unsafe notes file: {error}"
                )));
            }
        };

        let metadata = file.metadata().map_err(|error| {
            NotesError::repository(format!("could not inspect opened notes file: {error}"))
        })?;
        let mode = metadata.permissions().mode() & 0o7777;
        if !metadata.file_type().is_file() || mode != 0o600 {
            return Err(NotesError::repository(
                "refusing unsafe notes file: expected a regular 0600 file",
            ));
        }

        let mut contents = Vec::new();
        Read::by_ref(&mut file)
            .take((NOTES_FILE_MAX_BYTES + 1) as u64)
            .read_to_end(&mut contents)
            .map_err(|error| {
                NotesError::repository(format!("could not read notes file: {error}"))
            })?;
        if contents.len() > NOTES_FILE_MAX_BYTES {
            return Err(NotesError::repository(format!(
                "notes file is too large (maximum {NOTES_FILE_MAX_BYTES} bytes)"
            )));
        }

        let document = serde_json::from_slice::<NotesDocument>(&contents)?;
        document.validate()?;
        Ok(document)
    }

    fn save(&self, document: &NotesDocument) -> Result<(), NotesError> {
        self.save_with_writer(document, &FileAtomicWriter)
    }
}

pub(super) fn notes_path(xdg_data_home: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
        let path = PathBuf::from(value.filter(|value| !value.is_empty())?);
        path.is_absolute().then_some(path)
    }

    absolute(xdg_data_home)
        .or_else(|| absolute(home).map(|home| home.join(".local/share")))
        .map(|root| root.join("overcrow/notes/global.json"))
        .unwrap_or_default()
}

pub(super) trait AtomicWriter {
    fn write(&self, temporary: &mut NamedTempFile, contents: &[u8]) -> io::Result<()>;
    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()>;
    fn sync_parent(&self, parent: &Path) -> io::Result<()>;
}

struct FileAtomicWriter;

impl AtomicWriter for FileAtomicWriter {
    fn write(&self, temporary: &mut NamedTempFile, contents: &[u8]) -> io::Result<()> {
        temporary.write_all(contents)
    }

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        temporary
            .persist(destination)
            .map(|_| ())
            .map_err(|error| error.error)
    }

    fn sync_parent(&self, parent: &Path) -> io::Result<()> {
        fs::File::open(parent)?.sync_all()
    }
}
