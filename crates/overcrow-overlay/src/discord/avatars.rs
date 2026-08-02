use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{Cursor, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use eframe::egui::{self, TextureHandle};
use overcrow_logging::EventLogger;

use crate::runtime::widget_diagnostics::{FailureCategory, Provider, ProviderDiagnostics};

const CDN_BASE: &str = "https://cdn.discordapp.com/avatars";
const USER_AGENT: &str = concat!("PlayerVox-OverCrow/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const FAILED_RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
const REQUEST_CAPACITY: usize = 32;
const RESULT_CAPACITY: usize = REQUEST_CAPACITY + 1;
const CACHE_CAPACITY: usize = 64;
const ENCODED_IMAGE_MAX_BYTES: u64 = 256 * 1024;
const DECODED_IMAGE_MAX_EDGE: u32 = 256;
const DECODED_IMAGE_MAX_PIXELS: u32 = 256 * 256;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AvatarKey {
    user_id: String,
    avatar_hash: String,
}

impl AvatarKey {
    pub fn new(user_id: &str, avatar_hash: &str) -> Result<Self, AvatarError> {
        if !valid_user_id(user_id) || !valid_avatar_hash(avatar_hash) {
            return Err(AvatarError::InvalidKey);
        }
        Ok(Self {
            user_id: user_id.to_owned(),
            avatar_hash: avatar_hash.to_owned(),
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct DecodedAvatar {
    pub(super) size: [usize; 2],
    pub(super) rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AvatarError {
    InvalidKey,
    Transport,
    Response,
    TooLarge,
    Decode,
}

pub(super) trait AvatarFetcher: Send {
    fn fetch(&mut self, key: &AvatarKey) -> Result<DecodedAvatar, AvatarError>;
}

struct LiveAvatarFetcher {
    agent: ureq::Agent,
}

impl Default for LiveAvatarFetcher {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(REQUEST_TIMEOUT))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }
}

impl AvatarFetcher for LiveAvatarFetcher {
    fn fetch(&mut self, key: &AvatarKey) -> Result<DecodedAvatar, AvatarError> {
        let url = avatar_url(key)?;
        let mut response = self
            .agent
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "image/png")
            .call()
            .map_err(|_| AvatarError::Transport)?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(AvatarError::Response);
        }
        let is_png = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("image/png"));
        if !is_png {
            return Err(AvatarError::Response);
        }
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(ENCODED_IMAGE_MAX_BYTES.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|_| AvatarError::Transport)?;
        if body.len() as u64 > ENCODED_IMAGE_MAX_BYTES {
            return Err(AvatarError::TooLarge);
        }
        decode_png(&body)
    }
}

enum CacheEntry {
    Ready(TextureHandle),
    Failed(Instant),
}

struct FetchRequest {
    generation: u64,
    key: AvatarKey,
}

struct FetchResult {
    generation: u64,
    key: AvatarKey,
    result: Result<DecodedAvatar, AvatarError>,
}

pub struct DiscordAvatars {
    requests: Option<SyncSender<FetchRequest>>,
    results: Receiver<FetchResult>,
    shutdown: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
    pending: HashSet<AvatarKey>,
    referenced: HashSet<AvatarKey>,
    entries: HashMap<AvatarKey, CacheEntry>,
    lru: VecDeque<AvatarKey>,
    diagnostics: ProviderDiagnostics,
}

impl DiscordAvatars {
    pub fn spawn(logger: EventLogger, request_repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Self::spawn_fetcher(LiveAvatarFetcher::default(), logger, request_repaint)
    }

    #[cfg(test)]
    pub(super) fn spawn_with_fetcher(
        fetcher: impl AvatarFetcher + 'static,
        request_repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::spawn_fetcher(fetcher, EventLogger::disabled(), request_repaint)
    }

    fn spawn_fetcher(
        mut fetcher: impl AvatarFetcher + 'static,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (request_sender, requests) = mpsc::sync_channel::<FetchRequest>(REQUEST_CAPACITY);
        let (result_sender, results) = mpsc::sync_channel::<FetchResult>(RESULT_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let enabled = Arc::new(AtomicBool::new(false));
        let worker_enabled = Arc::clone(&enabled);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        let mut diagnostics = ProviderDiagnostics::new(logger, Provider::Discord);
        let worker = thread::Builder::new()
            .name("overcrow-discord-avatars".to_owned())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    if worker_shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    if !worker_enabled.load(Ordering::SeqCst)
                        || request.generation != worker_generation.load(Ordering::SeqCst)
                    {
                        continue;
                    }
                    let result = fetcher.fetch(&request.key);
                    if worker_shutdown.load(Ordering::SeqCst)
                        || !worker_enabled.load(Ordering::SeqCst)
                        || request.generation != worker_generation.load(Ordering::SeqCst)
                    {
                        continue;
                    }
                    if result_sender
                        .try_send(FetchResult {
                            generation: request.generation,
                            key: request.key,
                            result,
                        })
                        .is_ok()
                    {
                        request_repaint();
                    }
                }
            });
        let (requests, worker) = match worker {
            Ok(worker) => (Some(request_sender), Some(worker)),
            Err(_) => {
                diagnostics.failed(FailureCategory::Startup);
                (None, None)
            }
        };
        Self {
            requests,
            results,
            shutdown,
            enabled,
            generation,
            worker,
            pending: HashSet::new(),
            referenced: HashSet::new(),
            entries: HashMap::new(),
            lru: VecDeque::new(),
            diagnostics,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled.swap(enabled, Ordering::SeqCst) == enabled {
            return;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel_pending();
    }

    pub fn reset(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.referenced.clear();
        self.clear_transient();
    }

    pub fn retain_referenced<'a>(&mut self, keys: impl Iterator<Item = &'a AvatarKey>) {
        self.referenced = keys.take(CACHE_CAPACITY).cloned().collect();
        self.entries.retain(|key, _| self.referenced.contains(key));
        self.lru.retain(|key| self.entries.contains_key(key));
    }

    pub fn poll(&mut self, context: &egui::Context, now: Instant) {
        loop {
            let result = match self.results.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending.clear();
                    self.requests.take();
                    break;
                }
            };
            if result.generation != self.generation.load(Ordering::SeqCst)
                || !self.pending.remove(&result.key)
                || !self.referenced.contains(&result.key)
            {
                continue;
            }
            let entry = match result.result {
                Ok(decoded) => {
                    self.diagnostics.recovered();
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied(decoded.size, &decoded.rgba);
                    CacheEntry::Ready(context.load_texture(
                        "discord-avatar",
                        image,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                Err(_) => {
                    self.diagnostics.failed(FailureCategory::Response);
                    CacheEntry::Failed(now)
                }
            };
            self.insert(result.key, entry);
        }
    }

    pub fn texture(&mut self, key: &AvatarKey, now: Instant) -> Option<TextureHandle> {
        if !self.enabled.load(Ordering::SeqCst) || !self.referenced.contains(key) {
            return None;
        }
        match self.entries.get(key) {
            Some(CacheEntry::Ready(texture)) => {
                let texture = texture.clone();
                self.touch(key);
                return Some(texture);
            }
            Some(CacheEntry::Failed(failed_at))
                if now.saturating_duration_since(*failed_at) < FAILED_RETRY_AFTER =>
            {
                return None;
            }
            Some(CacheEntry::Failed(_)) | None => {}
        }
        if self.pending.len() >= REQUEST_CAPACITY || self.pending.contains(key) {
            return None;
        }
        let Some(requests) = &self.requests else {
            return None;
        };
        let request = FetchRequest {
            generation: self.generation.load(Ordering::SeqCst),
            key: key.clone(),
        };
        if requests.try_send(request).is_ok() {
            self.pending.insert(key.clone());
        }
        None
    }

    fn insert(&mut self, key: AvatarKey, entry: CacheEntry) {
        if !self.entries.contains_key(&key)
            && self.entries.len() == CACHE_CAPACITY
            && let Some(expired) = self.lru.pop_front()
        {
            self.entries.remove(&expired);
        }
        self.entries.insert(key.clone(), entry);
        self.touch(&key);
    }

    fn touch(&mut self, key: &AvatarKey) {
        self.lru.retain(|current| current != key);
        self.lru.push_back(key.clone());
    }

    fn clear_transient(&mut self) {
        self.cancel_pending();
        self.entries.clear();
        self.lru.clear();
    }

    fn cancel_pending(&mut self) {
        self.pending.clear();
        while self.results.try_recv().is_ok() {}
    }

    #[cfg(test)]
    pub(super) fn cached_len(&self) -> usize {
        self.entries.len()
    }
}

impl Drop for DiscordAvatars {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.requests.take();
        // Fetches have a hard timeout. Detach rather than block the egui thread.
        self.worker.take();
    }
}

pub(super) fn avatar_url(key: &AvatarKey) -> Result<String, AvatarError> {
    if !valid_user_id(&key.user_id) || !valid_avatar_hash(&key.avatar_hash) {
        return Err(AvatarError::InvalidKey);
    }
    Ok(format!(
        "{CDN_BASE}/{}/{}.png?size=64",
        key.user_id, key.avatar_hash
    ))
}

pub(super) fn decode_png(bytes: &[u8]) -> Result<DecodedAvatar, AvatarError> {
    let dimensions = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png)
        .into_dimensions()
        .map_err(|_| AvatarError::Decode)?;
    validate_dimensions(dimensions.0, dimensions.1)?;
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|_| AvatarError::Decode)?;
    let width = decoded.width();
    let height = decoded.height();
    validate_dimensions(width, height)?;
    Ok(DecodedAvatar {
        size: [width as usize, height as usize],
        rgba: decoded.into_rgba8().into_raw(),
    })
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), AvatarError> {
    let pixels = width.checked_mul(height).ok_or(AvatarError::TooLarge)?;
    if width == 0
        || height == 0
        || width > DECODED_IMAGE_MAX_EDGE
        || height > DECODED_IMAGE_MAX_EDGE
        || pixels > DECODED_IMAGE_MAX_PIXELS
    {
        return Err(AvatarError::TooLarge);
    }
    Ok(())
}

fn valid_user_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_avatar_hash(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
