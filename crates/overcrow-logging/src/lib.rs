use std::{
    collections::VecDeque,
    env,
    ffi::OsStr,
    fmt::{self, Write as _},
    fs::{self, File},
    io::{self, BufRead, BufReader, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

use chrono::{DateTime, SecondsFormat, Utc};

pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_LINE_BYTES: usize = 1024;
pub const QUEUE_CAPACITY: usize = 256;
pub const ARCHIVE_COUNT: usize = 3;
pub const DEFAULT_READ_LINES: usize = 2_000;

const MAX_EVENT_NAME_BYTES: usize = 64;
const MAX_DETAILS_BYTES: usize = 768;
const OPEN_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK;
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(50);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Component {
    Core,
    Overlay,
    Hyprland,
}

impl Component {
    const ALL: [Self; 3] = [Self::Core, Self::Overlay, Self::Hyprland];

    fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Overlay => "overlay",
            Self::Hyprland => "hyprland",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Core => "core.log",
            Self::Overlay => "overlay.log",
            Self::Hyprland => "hyprland.log",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

struct QueuedEvent {
    timestamp: SystemTime,
    level: Level,
    event: &'static str,
    details: String,
}

#[derive(Clone, Default)]
pub struct EventLogger {
    sender: Option<SyncSender<QueuedEvent>>,
    dropped: Arc<AtomicU64>,
}

impl EventLogger {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn info(&self, event: &'static str, details: fmt::Arguments<'_>) {
        self.enqueue(Level::Info, event, details);
    }

    pub fn warn(&self, event: &'static str, details: fmt::Arguments<'_>) {
        self.enqueue(Level::Warn, event, details);
    }

    pub fn error(&self, event: &'static str, details: fmt::Arguments<'_>) {
        self.enqueue(Level::Error, event, details);
    }

    fn enqueue(&self, level: Level, event: &'static str, details: fmt::Arguments<'_>) {
        let Some(sender) = &self.sender else {
            return;
        };
        let event = validated_event_name(event);
        let mut sanitized = SanitizedText::new(MAX_DETAILS_BYTES);
        let _ = sanitized.write_fmt(details);
        let queued = QueuedEvent {
            timestamp: SystemTime::now(),
            level,
            event,
            details: sanitized.finish(),
        };
        match sender.try_send(queued) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                let _ = self
                    .dropped
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        Some(value.saturating_add(1))
                    });
            }
        }
    }
}

pub struct LoggerRuntime {
    logger: EventLogger,
    shutdown: Arc<AtomicBool>,
    stopped: Receiver<()>,
    worker: Option<JoinHandle<()>>,
}

impl LoggerRuntime {
    pub fn start(component: Component) -> io::Result<Self> {
        let directory = log_directory_from_values(
            env::var_os("XDG_STATE_HOME").as_deref(),
            env::var_os("HOME").as_deref(),
        );
        Self::start_in(component, directory)
    }

    pub fn start_in(component: Component, directory: impl Into<PathBuf>) -> io::Result<Self> {
        let directory = directory.into();
        let sink = match FileSink::open(component, &directory) {
            Ok(sink) => Sink::File(sink),
            Err(error) => {
                eprintln!(
                    "OverCrow diagnostic file unavailable for {}: {error}; using stderr",
                    component.name()
                );
                Sink::Stderr
            }
        };
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_dropped = Arc::clone(&dropped);
        let (stopped_sender, stopped) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name(format!("overcrow-{}-log", component.name()))
            .spawn(move || {
                run_worker(component, receiver, worker_dropped, worker_shutdown, sink);
                let _ = stopped_sender.try_send(());
            })?;
        Ok(Self {
            logger: EventLogger {
                sender: Some(sender),
                dropped,
            },
            shutdown,
            stopped,
            worker: Some(worker),
        })
    }

    pub fn logger(&self) -> EventLogger {
        self.logger.clone()
    }
}

impl Drop for LoggerRuntime {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            match self.stopped.recv_timeout(WORKER_SHUTDOWN_TIMEOUT) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = worker.join();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Detach rather than letting a slow filesystem stall process shutdown.
                }
            }
        }
    }
}

enum Sink {
    File(FileSink),
    Stderr,
}

impl Sink {
    fn write(&mut self, line: &str) {
        if let Self::File(file) = self
            && let Err(error) = file.write(line.as_bytes())
        {
            eprintln!("OverCrow diagnostic log write failed: {error}; using stderr");
            *self = Self::Stderr;
        }
        if matches!(self, Self::Stderr) {
            eprint!("{line}");
        }
    }
}

struct FileSink {
    path: PathBuf,
    file: Option<File>,
    size: u64,
}

impl FileSink {
    fn open(component: Component, directory: &Path) -> io::Result<Self> {
        if directory.as_os_str().is_empty() || !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "diagnostic log directory is unavailable",
            ));
        }
        let controlled_parent = directory
            .parent()
            .filter(|_| directory.file_name() == Some(OsStr::new("logs")))
            .filter(|parent| parent.file_name() == Some(OsStr::new("overcrow")));
        if let Some(parent) = controlled_parent {
            ensure_private_directory(parent)?;
        }
        ensure_private_directory(directory)?;
        let path = directory.join(component.file_name());
        validate_archives(&path)?;
        let (file, size) = open_log_file(&path)?;
        Ok(Self {
            path,
            file: Some(file),
            size,
        })
    }

    fn write(&mut self, line: &[u8]) -> io::Result<()> {
        let line_len = u64::try_from(line.len())
            .map_err(|_| io::Error::other("diagnostic line length overflow"))?;
        if self
            .size
            .checked_add(line_len)
            .is_none_or(|size| size > MAX_FILE_BYTES)
        {
            self.rotate()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("diagnostic log file is unavailable"))?;
        file.write_all(line)?;
        self.size = self
            .size
            .checked_add(line_len)
            .ok_or_else(|| io::Error::other("diagnostic log size overflow"))?;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        rotate_files(&self.path)?;
        let (file, size) = open_log_file(&self.path)?;
        self.file = Some(file);
        self.size = size;
        Ok(())
    }
}

fn run_worker(
    component: Component,
    receiver: Receiver<QueuedEvent>,
    dropped: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    mut sink: Sink,
) {
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(event) => write_event(component, &event, &dropped, &mut sink),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    for _ in 0..QUEUE_CAPACITY {
        match receiver.try_recv() {
            Ok(event) => write_event(component, &event, &dropped, &mut sink),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
    write_dropped(component, SystemTime::now(), &dropped, &mut sink);
}

fn write_event(component: Component, event: &QueuedEvent, dropped: &AtomicU64, sink: &mut Sink) {
    write_dropped(component, event.timestamp, dropped, sink);
    sink.write(&format_line(component, event));
}

fn write_dropped(
    component: Component,
    timestamp: SystemTime,
    dropped: &AtomicU64,
    sink: &mut Sink,
) {
    let count = dropped.swap(0, Ordering::Relaxed);
    if count == 0 {
        return;
    }
    sink.write(&format_line(
        component,
        &QueuedEvent {
            timestamp,
            level: Level::Warn,
            event: "events_dropped",
            details: format!("count={count}"),
        },
    ));
}

fn format_line(component: Component, event: &QueuedEvent) -> String {
    let mut line = String::with_capacity(MAX_LINE_BYTES);
    let _ = write!(
        line,
        "{} {} {} {}",
        timestamp_at(event.timestamp),
        event.level.name(),
        component.name(),
        event.event
    );
    if !event.details.is_empty() {
        line.push(' ');
        append_bounded(&mut line, &event.details, MAX_LINE_BYTES - 1);
    }
    if line.len() >= MAX_LINE_BYTES {
        truncate_utf8(&mut line, MAX_LINE_BYTES - 1);
    }
    line.push('\n');
    line
}

fn timestamp_at(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn validated_event_name(event: &'static str) -> &'static str {
    if !event.is_empty()
        && event.len() <= MAX_EVENT_NAME_BYTES
        && event
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        event
    } else {
        "invalid_event_name"
    }
}

struct SanitizedText {
    output: String,
    maximum: usize,
}

impl SanitizedText {
    fn new(maximum: usize) -> Self {
        Self {
            output: String::with_capacity(maximum),
            maximum,
        }
    }

    fn finish(self) -> String {
        self.output
    }

    fn append(&mut self, value: &str) {
        append_bounded(&mut self.output, value, self.maximum);
    }
}

impl fmt::Write for SanitizedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for character in value.chars() {
            match character {
                '\n' => self.append("\\n"),
                '\r' => self.append("\\r"),
                '\t' => self.append("\\t"),
                character if character.is_control() => {
                    let escaped = format!("\\u{{{:x}}}", u32::from(character));
                    self.append(&escaped);
                }
                character => {
                    let mut encoded = [0_u8; 4];
                    self.append(character.encode_utf8(&mut encoded));
                }
            }
            if self.output.len() >= self.maximum {
                break;
            }
        }
        Ok(())
    }
}

fn append_bounded(output: &mut String, value: &str, maximum: usize) {
    if output.len() >= maximum {
        return;
    }
    let available = maximum - output.len();
    let mut end = value.len().min(available);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&value[..end]);
}

fn truncate_utf8(value: &mut String, maximum: usize) {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

fn log_directory_from_values(xdg_state_home: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    if let Some(root) = xdg_state_home
        .map(Path::new)
        .filter(|root| root.is_absolute())
    {
        return root.join("overcrow/logs");
    }
    home.map(Path::new)
        .filter(|root| root.is_absolute())
        .map(|root| root.join(".local/state/overcrow/logs"))
        .unwrap_or_default()
}

fn ensure_private_directory(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "diagnostic log directory is not a regular directory",
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn open_log_file(path: &Path) -> io::Result<(File, u64)> {
    let existed = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_private_file_metadata(&metadata)?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(OPEN_FLAGS)
        .open(path)?;
    if !existed {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    let metadata = file.metadata()?;
    validate_private_file_metadata(&metadata)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "diagnostic log exceeds its size limit",
        ));
    }
    Ok((file, metadata.len()))
}

fn validate_private_file_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o7777 != 0o600 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "diagnostic log must be a regular 0600 file",
        ));
    }
    Ok(())
}

fn archive_path(path: &Path, generation: usize) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".{generation}"));
    PathBuf::from(value)
}

fn validate_archives(path: &Path) -> io::Result<()> {
    for generation in 1..=ARCHIVE_COUNT {
        let archive = archive_path(path, generation);
        match fs::symlink_metadata(&archive) {
            Ok(metadata) => {
                validate_private_file_metadata(&metadata)?;
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "diagnostic archive exceeds its size limit",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn rotate_files(path: &Path) -> io::Result<()> {
    validate_archives(path)?;
    let oldest = archive_path(path, ARCHIVE_COUNT);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }
    for generation in (1..ARCHIVE_COUNT).rev() {
        let source = archive_path(path, generation);
        if source.exists() {
            fs::rename(source, archive_path(path, generation + 1))?;
        }
    }
    if path.exists() {
        fs::rename(path, archive_path(path, 1))?;
    }
    Ok(())
}

pub fn read_recent_logs(limit: usize) -> io::Result<Vec<String>> {
    let directory = log_directory_from_values(
        env::var_os("XDG_STATE_HOME").as_deref(),
        env::var_os("HOME").as_deref(),
    );
    if directory.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "diagnostic log directory is unavailable",
        ));
    }
    read_recent_logs_in(&directory, limit)
}

fn read_recent_logs_in(directory: &Path, limit: usize) -> io::Result<Vec<String>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    match fs::symlink_metadata(directory) {
        Ok(metadata) => validate_private_directory_metadata(&metadata)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    }
    if directory.file_name() == Some(OsStr::new("logs"))
        && let Some(parent) = directory
            .parent()
            .filter(|parent| parent.file_name() == Some(OsStr::new("overcrow")))
    {
        validate_private_directory_metadata(&fs::symlink_metadata(parent)?)?;
    }
    let mut entries = Vec::new();
    for component in Component::ALL {
        let path = directory.join(component.file_name());
        let mut component_lines = VecDeque::with_capacity(limit.min(256));
        for generation in (1..=ARCHIVE_COUNT).rev() {
            read_safe_lines(
                &archive_path(&path, generation),
                limit,
                &mut component_lines,
            )?;
        }
        read_safe_lines(&path, limit, &mut component_lines)?;
        entries.extend(component_lines);
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let skip = entries.len().saturating_sub(limit);
    Ok(entries
        .into_iter()
        .skip(skip)
        .map(|(_, line)| line)
        .collect())
}

fn validate_private_directory_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o7777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "diagnostic log directory must be a regular 0700 directory",
        ));
    }
    Ok(())
}

fn read_safe_lines(
    path: &Path,
    limit: usize,
    lines: &mut VecDeque<(DateTime<Utc>, String)>,
) -> io::Result<()> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_FLAGS)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    validate_private_file_metadata(&metadata)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "diagnostic log exceeds its size limit",
        ));
    }
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.len().saturating_add(1) > MAX_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "diagnostic line exceeds its size limit",
            ));
        }
        let (timestamp, _) = line.split_once(' ').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "diagnostic timestamp is missing",
            )
        })?;
        let timestamp = timestamp
            .parse::<DateTime<Utc>>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid timestamp"))?;
        if lines.len() == limit {
            lines.pop_front();
        }
        lines.push_back((timestamp, line));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use chrono::{DateTime, Utc};

    use super::*;

    fn private_file(path: &std::path::Path, contents: &str) {
        fs::write(path, contents).expect("write fixture");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("make fixture private");
    }

    #[test]
    fn writes_timestamped_private_bounded_lines() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let runtime =
            LoggerRuntime::start_in(Component::Core, temp.path()).expect("start logger runtime");
        runtime.logger().info(
            "game_detected",
            format_args!(
                "value=line\nforgery payload={}",
                "x".repeat(MAX_LINE_BYTES * 2)
            ),
        );
        drop(runtime);

        let path = temp.path().join("core.log");
        let line = fs::read_to_string(&path).expect("read log line");
        let (timestamp, message) = line.split_once(' ').expect("timestamp separator");
        DateTime::parse_from_rfc3339(timestamp).expect("RFC3339 timestamp");
        assert!(timestamp.ends_with('Z'));
        assert!(message.starts_with("INFO core game_detected "));
        assert!(message.contains("value=line\\nforgery"));
        assert!(!message.contains("value=line\nforgery"));
        assert!(line.len() <= MAX_LINE_BYTES);
        assert!(line.ends_with('\n'));
        assert_eq!(
            fs::metadata(path)
                .expect("log metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn rotates_at_the_size_limit_and_keeps_three_archives() {
        let temp = tempfile::tempdir().expect("create temp directory");
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
            .expect("make directory private");
        private_file(
            &temp.path().join("core.log"),
            &"a".repeat(MAX_FILE_BYTES as usize),
        );
        private_file(&temp.path().join("core.log.1"), "one\n");
        private_file(&temp.path().join("core.log.2"), "two\n");
        private_file(&temp.path().join("core.log.3"), "three\n");

        let runtime =
            LoggerRuntime::start_in(Component::Core, temp.path()).expect("start logger runtime");
        runtime
            .logger()
            .info("rotation_probe", format_args!("sequence=4"));
        drop(runtime);

        assert!(
            fs::read_to_string(temp.path().join("core.log"))
                .expect("current log")
                .contains("rotation_probe")
        );
        assert_eq!(
            fs::metadata(temp.path().join("core.log.1"))
                .expect("first archive")
                .len(),
            MAX_FILE_BYTES
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("core.log.2")).expect("second archive"),
            "one\n"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("core.log.3")).expect("third archive"),
            "two\n"
        );
        assert!(!temp.path().join("core.log.4").exists());
    }

    #[test]
    fn refuses_a_symlinked_log_without_touching_its_target() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let target = temp.path().join("target");
        private_file(&target, "keep\n");
        symlink(&target, temp.path().join("core.log")).expect("create symlink");

        let runtime =
            LoggerRuntime::start_in(Component::Core, temp.path()).expect("start stderr fallback");
        runtime
            .logger()
            .error("unsafe_path", format_args!("fallback=true"));
        drop(runtime);

        assert_eq!(
            fs::read_to_string(target).expect("target content"),
            "keep\n"
        );
    }

    #[test]
    fn refuses_a_symlinked_controlled_parent_before_creating_the_log_directory() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("create target directory");
        symlink(&target, temp.path().join("overcrow")).expect("create parent symlink");

        let runtime =
            LoggerRuntime::start_in(Component::Core, temp.path().join("overcrow").join("logs"))
                .expect("start stderr fallback");
        runtime
            .logger()
            .info("process_started", format_args!("fallback=true"));
        drop(runtime);

        assert!(!target.join("logs").exists());
    }

    #[test]
    fn saturated_queue_drops_immediately_instead_of_blocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let logger = EventLogger {
            sender: Some(sender),
            dropped: Arc::clone(&dropped),
        };
        logger.info("first", format_args!("value=1"));

        let started = Instant::now();
        logger.info("second", format_args!("value=2"));

        assert!(started.elapsed() < Duration::from_millis(50));
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn shutdown_does_not_wait_indefinitely_for_a_stalled_worker() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (stopped_sender, stopped) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(750));
            drop(stopped_sender);
        });
        let runtime = LoggerRuntime {
            logger: EventLogger::disabled(),
            shutdown,
            stopped,
            worker: Some(worker),
        };

        let started = Instant::now();
        drop(runtime);

        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn worker_reports_a_prior_dropped_event_before_the_next_entry() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let runtime =
            LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start logger runtime");
        runtime.logger().dropped.store(3, Ordering::Relaxed);
        runtime
            .logger()
            .info("core_connected", format_args!("generation=2"));
        drop(runtime);

        let contents = fs::read_to_string(temp.path().join("overlay.log")).expect("read log");
        assert!(contents.contains("WARN overlay events_dropped count=3"));
        assert!(contents.contains("INFO overlay core_connected generation=2"));
    }

    #[test]
    fn recent_reader_merges_components_and_keeps_only_the_newest_limit() {
        let temp = tempfile::tempdir().expect("create temp directory");
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
            .expect("make directory private");
        private_file(
            &temp.path().join("core.log"),
            "2026-07-20T10:00:00.000Z INFO core first\n2026-07-20T10:00:02.000Z INFO core third\n",
        );
        private_file(
            &temp.path().join("overlay.log"),
            "2026-07-20T10:00:01.000Z INFO overlay second\n",
        );

        assert_eq!(
            read_recent_logs_in(temp.path(), 2).expect("read recent logs"),
            vec![
                "2026-07-20T10:00:01.000Z INFO overlay second".to_owned(),
                "2026-07-20T10:00:02.000Z INFO core third".to_owned(),
            ]
        );
    }

    #[test]
    fn recent_reader_refuses_unsafe_or_oversized_files() {
        let temp = tempfile::tempdir().expect("create temp directory");
        private_file(&temp.path().join("target"), "safe\n");
        symlink(temp.path().join("target"), temp.path().join("hyprland.log"))
            .expect("create symlink");
        assert!(read_recent_logs_in(temp.path(), 10).is_err());

        fs::remove_file(temp.path().join("hyprland.log")).expect("remove symlink");
        private_file(
            &temp.path().join("hyprland.log"),
            &"x".repeat(MAX_FILE_BYTES as usize + 1),
        );
        assert!(read_recent_logs_in(temp.path(), 10).is_err());
    }

    #[test]
    fn recent_reader_refuses_a_symlinked_log_directory() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("create target directory");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
            .expect("make target private");
        private_file(
            &target.join("core.log"),
            "2026-07-20T10:00:00.000Z INFO core event\n",
        );
        let linked = temp.path().join("logs");
        symlink(&target, &linked).expect("create log directory symlink");

        assert!(read_recent_logs_in(&linked, 10).is_err());
    }

    #[test]
    fn state_path_requires_an_absolute_xdg_or_home_root() {
        assert_eq!(
            log_directory_from_values(Some(std::ffi::OsStr::new("/state")), None),
            std::path::PathBuf::from("/state/overcrow/logs")
        );
        assert_eq!(
            log_directory_from_values(
                Some(std::ffi::OsStr::new("relative")),
                Some(std::ffi::OsStr::new("/home/player")),
            ),
            std::path::PathBuf::from("/home/player/.local/state/overcrow/logs")
        );
        assert!(log_directory_from_values(None, None).as_os_str().is_empty());
    }

    #[test]
    fn timestamp_fixture_uses_utc() {
        let timestamp = timestamp_at(std::time::UNIX_EPOCH);
        assert_eq!(timestamp, "1970-01-01T00:00:00.000Z");
        assert_eq!(timestamp.parse::<DateTime<Utc>>().unwrap().timestamp(), 0);
    }
}
