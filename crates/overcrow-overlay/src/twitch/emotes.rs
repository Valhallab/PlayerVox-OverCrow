use std::{
    collections::{HashMap, HashSet},
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

use super::model::valid_twitch_emote_id;

const CDN_BASE: &str = "https://static-cdn.jtvnw.net/emoticons/v2";
const USER_AGENT: &str = concat!("PlayerVox-OverCrow/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const FAILED_RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
const REQUEST_CAPACITY: usize = 32;
// A reset can race with the single in-flight fetch after its final generation
// check. Keep one slot beyond the tracked request bound for that stale result.
const RESULT_CAPACITY: usize = REQUEST_CAPACITY + 1;
const CACHE_CAPACITY: usize = 128;
const ENCODED_IMAGE_MAX_BYTES: u64 = 128 * 1024;
const DECODED_IMAGE_MAX_WIDTH: u32 = 256;
const DECODED_IMAGE_MAX_HEIGHT: u32 = 112;
const DECODED_IMAGE_MAX_PIXELS: u32 = 256 * 112;

#[derive(Debug)]
struct DecodedEmote {
    size: [usize; 2],
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EmoteError {
    InvalidId,
    Transport,
    Response,
    TooLarge,
    Decode,
}

trait EmoteFetcher: Send {
    fn fetch(&mut self, id: &str) -> Result<DecodedEmote, EmoteError>;
}

struct LiveEmoteFetcher {
    agent: ureq::Agent,
}

impl Default for LiveEmoteFetcher {
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

impl EmoteFetcher for LiveEmoteFetcher {
    fn fetch(&mut self, id: &str) -> Result<DecodedEmote, EmoteError> {
        let url = emote_url(id)?;
        let mut response = self
            .agent
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "image/png")
            .call()
            .map_err(|_| EmoteError::Transport)?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(EmoteError::Response);
        }
        let is_png = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("image/png"));
        if !is_png {
            return Err(EmoteError::Response);
        }

        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(ENCODED_IMAGE_MAX_BYTES.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|_| EmoteError::Transport)?;
        if body.len() as u64 > ENCODED_IMAGE_MAX_BYTES {
            return Err(EmoteError::TooLarge);
        }
        decode_png(&body)
    }
}

enum CacheEntry {
    Ready(TextureHandle),
    Failed(Instant),
}

struct FetchResult {
    generation: u64,
    id: String,
    result: Result<DecodedEmote, EmoteError>,
}

struct FetchRequest {
    generation: u64,
    id: String,
}

pub struct TwitchEmotes {
    requests: Option<SyncSender<FetchRequest>>,
    results: Receiver<FetchResult>,
    shutdown: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
    pending: HashSet<String>,
    referenced: HashSet<String>,
    entries: HashMap<String, CacheEntry>,
    diagnostics: ProviderDiagnostics,
}

impl TwitchEmotes {
    pub fn spawn(logger: EventLogger, request_repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Self::spawn_with_fetcher(LiveEmoteFetcher::default(), logger, request_repaint)
    }

    fn spawn_with_fetcher<F>(
        mut fetcher: F,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self
    where
        F: EmoteFetcher + 'static,
    {
        let (request_sender, requests) = mpsc::sync_channel::<FetchRequest>(REQUEST_CAPACITY);
        let (result_sender, results) = mpsc::sync_channel::<FetchResult>(RESULT_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let enabled = Arc::new(AtomicBool::new(false));
        let worker_enabled = Arc::clone(&enabled);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        let mut diagnostics = ProviderDiagnostics::new(logger, Provider::Twitch);
        let worker = thread::Builder::new()
            .name("overcrow-twitch-emotes".to_owned())
            .spawn(move || {
                'worker: while let Ok(mut request) = requests.recv() {
                    loop {
                        if worker_shutdown.load(Ordering::SeqCst) {
                            break 'worker;
                        }
                        if worker_enabled.load(Ordering::SeqCst)
                            && request.generation == worker_generation.load(Ordering::SeqCst)
                        {
                            break;
                        }
                        request = match requests.try_recv() {
                            Ok(request) => request,
                            Err(TryRecvError::Empty) => {
                                if worker_enabled.load(Ordering::SeqCst) {
                                    request_repaint();
                                }
                                continue 'worker;
                            }
                            Err(TryRecvError::Disconnected) => break 'worker,
                        };
                    }
                    let result = fetcher.fetch(&request.id);
                    if worker_shutdown.load(Ordering::SeqCst)
                        || !worker_enabled.load(Ordering::SeqCst)
                        || request.generation != worker_generation.load(Ordering::SeqCst)
                    {
                        continue;
                    }
                    if result_sender
                        .try_send(FetchResult {
                            generation: request.generation,
                            id: request.id,
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
            diagnostics,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled.swap(enabled, Ordering::SeqCst) == enabled {
            return;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.clear_transient();
    }

    pub fn reset(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.referenced.clear();
        self.clear_transient();
    }

    pub fn retain_referenced<'a>(&mut self, ids: impl Iterator<Item = &'a str>) {
        let mut referenced = HashSet::with_capacity(CACHE_CAPACITY);
        for id in ids {
            referenced.insert(id.to_owned());
            if referenced.len() == CACHE_CAPACITY {
                break;
            }
        }
        self.referenced = referenced;
        self.entries
            .retain(|id, _| self.referenced.contains(id.as_str()));
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
                || !self.pending.remove(&result.id)
            {
                continue;
            }
            if !self.referenced.contains(&result.id) {
                continue;
            }
            let entry = match result.result {
                Ok(decoded) => {
                    self.diagnostics.recovered();
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied(decoded.size, &decoded.rgba);
                    CacheEntry::Ready(context.load_texture(
                        "twitch-emote",
                        image,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                Err(_) => {
                    self.diagnostics.failed(FailureCategory::Response);
                    CacheEntry::Failed(now)
                }
            };
            self.insert(result.id, entry);
        }
    }

    pub fn texture(&mut self, id: &str, now: Instant) -> Option<TextureHandle> {
        if !self.enabled.load(Ordering::SeqCst)
            || !self.referenced.contains(id)
            || !valid_twitch_emote_id(id)
        {
            return None;
        }
        match self.entries.get(id) {
            Some(CacheEntry::Ready(texture)) => return Some(texture.clone()),
            Some(CacheEntry::Failed(failed_at))
                if now.saturating_duration_since(*failed_at) < FAILED_RETRY_AFTER =>
            {
                return None;
            }
            Some(CacheEntry::Failed(_)) | None => {}
        }
        if !self.entries.contains_key(id) && self.entries.len() >= CACHE_CAPACITY {
            return None;
        }
        if self.pending.contains(id) {
            return None;
        }
        if self.pending.len() >= REQUEST_CAPACITY {
            return None;
        }
        let Some(requests) = &self.requests else {
            return None;
        };
        let request = FetchRequest {
            generation: self.generation.load(Ordering::SeqCst),
            id: id.to_owned(),
        };
        if requests.try_send(request).is_ok() {
            self.pending.insert(id.to_owned());
        }
        None
    }

    fn insert(&mut self, id: String, entry: CacheEntry) {
        if !self.entries.contains_key(&id) && self.entries.len() >= CACHE_CAPACITY {
            return;
        }
        self.entries.insert(id, entry);
    }

    fn clear_transient(&mut self) {
        self.pending.clear();
        self.entries.clear();
        while self.results.try_recv().is_ok() {}
    }

    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        let (_, results) = mpsc::sync_channel(1);
        Self {
            requests: None,
            results,
            shutdown: Arc::new(AtomicBool::new(false)),
            enabled: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            worker: None,
            pending: HashSet::new(),
            referenced: HashSet::new(),
            entries: HashMap::new(),
            diagnostics: ProviderDiagnostics::new(EventLogger::disabled(), Provider::Twitch),
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_test_texture(
        &mut self,
        context: &egui::Context,
        id: &str,
        image: egui::ColorImage,
    ) {
        self.insert(
            id.to_owned(),
            CacheEntry::Ready(context.load_texture(
                "twitch-emote-test",
                image,
                egui::TextureOptions::LINEAR,
            )),
        );
    }
}

impl Drop for TwitchEmotes {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn emote_url(id: &str) -> Result<String, EmoteError> {
    if !valid_twitch_emote_id(id) {
        return Err(EmoteError::InvalidId);
    }
    Ok(format!("{CDN_BASE}/{id}/static/dark/1.0"))
}

fn decode_png(bytes: &[u8]) -> Result<DecodedEmote, EmoteError> {
    let dimensions = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png)
        .into_dimensions()
        .map_err(|_| EmoteError::Decode)?;
    validate_dimensions(dimensions.0, dimensions.1)?;
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|_| EmoteError::Decode)?;
    let width = decoded.width();
    let height = decoded.height();
    validate_dimensions(width, height)?;
    let rgba = decoded.into_rgba8().into_raw();
    Ok(DecodedEmote {
        size: [width as usize, height as usize],
        rgba,
    })
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), EmoteError> {
    let pixels = width.checked_mul(height).ok_or(EmoteError::TooLarge)?;
    if width == 0
        || height == 0
        || width > DECODED_IMAGE_MAX_WIDTH
        || height > DECODED_IMAGE_MAX_HEIGHT
        || pixels > DECODED_IMAGE_MAX_PIXELS
    {
        return Err(EmoteError::TooLarge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use eframe::egui;
    use image::{DynamicImage, ImageFormat, RgbaImage};
    use overcrow_logging::EventLogger;

    use super::{DecodedEmote, EmoteError, EmoteFetcher, TwitchEmotes, decode_png, emote_url};

    #[test]
    fn emote_url_uses_only_the_fixed_twitch_cdn_shape() {
        assert_eq!(
            emote_url("emotesv2_abc-123").expect("valid ID"),
            "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_abc-123/static/dark/1.0"
        );
        for invalid in ["", "../escape", "a/b", "space here", &"x".repeat(129)] {
            assert_eq!(emote_url(invalid), Err(EmoteError::InvalidId));
        }
    }

    #[test]
    fn png_decoder_accepts_small_rgba_images_and_rejects_oversized_dimensions() {
        let valid = encode_png(28, 28);
        let decoded = decode_png(&valid).expect("valid PNG");
        assert_eq!(decoded.size, [28, 28]);
        assert_eq!(decoded.rgba.len(), 28 * 28 * 4);

        let oversized = encode_png(257, 1);
        assert_eq!(decode_png(&oversized).unwrap_err(), EmoteError::TooLarge);
        assert_eq!(decode_png(b"not a png").unwrap_err(), EmoteError::Decode);
    }

    #[test]
    fn asynchronous_loader_repaints_and_reuses_the_cached_texture() {
        struct StaticFetcher {
            calls: Arc<AtomicUsize>,
        }

        impl EmoteFetcher for StaticFetcher {
            fn fetch(&mut self, _id: &str) -> Result<DecodedEmote, EmoteError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(DecodedEmote {
                    size: [1, 1],
                    rgba: vec![255, 255, 255, 255],
                })
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let repaints = Arc::new(AtomicUsize::new(0));
        let repaint_counter = Arc::clone(&repaints);
        let mut emotes = TwitchEmotes::spawn_with_fetcher(
            StaticFetcher {
                calls: Arc::clone(&calls),
            },
            EventLogger::disabled(),
            move || {
                repaint_counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        emotes.set_enabled(true);
        emotes.retain_referenced(["425618"].into_iter());
        let context = egui::Context::default();
        let requested_at = Instant::now();
        assert!(emotes.texture("425618", requested_at).is_none());

        let deadline = Instant::now() + Duration::from_secs(1);
        let texture = loop {
            emotes.poll(&context, Instant::now());
            if let Some(texture) = emotes.texture("425618", Instant::now()) {
                break texture;
            }
            assert!(Instant::now() < deadline, "emote worker timed out");
            thread::sleep(Duration::from_millis(1));
        };

        assert_eq!(texture.size(), [1, 1]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(repaints.load(Ordering::SeqCst), 1);
        assert!(emotes.texture("425618", Instant::now()).is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn decoded_texture_cache_keeps_existing_entries_and_rejects_overflow() {
        let context = egui::Context::default();
        let image = egui::ColorImage::filled([1, 1], egui::Color32::WHITE);
        let mut emotes = TwitchEmotes::disabled();
        emotes.set_enabled(true);

        for index in 0..=super::CACHE_CAPACITY {
            emotes.insert_test_texture(&context, &index.to_string(), image.clone());
        }

        assert_eq!(emotes.entries.len(), super::CACHE_CAPACITY);
        assert!(emotes.entries.contains_key("0"));
        assert!(
            !emotes
                .entries
                .contains_key(&super::CACHE_CAPACITY.to_string())
        );

        let (requests, receiver) =
            mpsc::sync_channel::<super::FetchRequest>(super::REQUEST_CAPACITY);
        emotes.requests = Some(requests);
        for _ in 0..3 {
            assert!(emotes.texture("not-cached", Instant::now()).is_none());
        }
        assert!(receiver.try_recv().is_err());
        assert!(emotes.pending.is_empty());

        emotes.retain_referenced(["0", "not-cached"].into_iter());
        assert!(emotes.texture("not-cached", Instant::now()).is_none());
        assert_eq!(receiver.recv().expect("capacity released").id, "not-cached");
    }

    #[test]
    fn bounded_references_prioritize_recent_emotes_without_cache_starvation() {
        let context = egui::Context::default();
        let image = egui::ColorImage::filled([1, 1], egui::Color32::WHITE);
        let mut emotes = TwitchEmotes::disabled();
        emotes.set_enabled(true);
        let historical = (0..super::CACHE_CAPACITY)
            .map(|index| format!("old-{index}"))
            .collect::<Vec<_>>();
        let recent = (0..super::CACHE_CAPACITY)
            .map(|index| format!("new-{index}"))
            .collect::<Vec<_>>();
        for id in &historical {
            emotes.insert_test_texture(&context, id, image.clone());
        }

        emotes.retain_referenced(recent.iter().chain(&historical).map(String::as_str));

        assert_eq!(emotes.referenced.len(), super::CACHE_CAPACITY);
        assert!(emotes.entries.is_empty());
        let (requests, receiver) =
            mpsc::sync_channel::<super::FetchRequest>(super::REQUEST_CAPACITY);
        emotes.requests = Some(requests);
        assert!(emotes.texture(&historical[0], Instant::now()).is_none());
        assert!(receiver.try_recv().is_err());
        assert!(emotes.texture(&recent[0], Instant::now()).is_none());
        assert_eq!(receiver.recv().expect("recent emote request").id, recent[0]);
    }

    #[test]
    fn pending_requests_never_exceed_the_fixed_worker_capacity() {
        let (requests, receiver) =
            std::sync::mpsc::sync_channel::<super::FetchRequest>(super::REQUEST_CAPACITY);
        let now = Instant::now();
        let mut emotes = TwitchEmotes::disabled();
        emotes.set_enabled(true);
        emotes.requests = Some(requests);
        for index in 0..super::REQUEST_CAPACITY {
            emotes.pending.insert(index.to_string());
        }

        assert!(emotes.texture("425618", now).is_none());
        assert_eq!(emotes.pending.len(), super::REQUEST_CAPACITY);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn duplicate_pending_emote_is_not_enqueued_twice() {
        let (requests, receiver) =
            mpsc::sync_channel::<super::FetchRequest>(super::REQUEST_CAPACITY);
        let now = Instant::now();
        let mut emotes = TwitchEmotes::disabled();
        emotes.set_enabled(true);
        emotes.requests = Some(requests);
        emotes.retain_referenced(["425618"].into_iter());

        assert!(emotes.texture("425618", now).is_none());
        assert_eq!(receiver.recv().expect("first request").id, "425618");
        emotes.retain_referenced(std::iter::empty());
        assert!(emotes.pending.contains("425618"));
        emotes.retain_referenced(["425618"].into_iter());
        assert!(
            emotes
                .texture("425618", now + Duration::from_secs(60))
                .is_none()
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(emotes.pending.len(), 1);
    }

    #[test]
    fn drop_finishes_current_fetch_without_draining_queued_requests() {
        struct ControlledFetcher {
            calls: Arc<AtomicUsize>,
            started: mpsc::SyncSender<()>,
            release: mpsc::Receiver<()>,
        }

        impl EmoteFetcher for ControlledFetcher {
            fn fetch(&mut self, _id: &str) -> Result<DecodedEmote, EmoteError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let _ = self.started.try_send(());
                self.release.recv().expect("release current fetch");
                Err(EmoteError::Transport)
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let (started_sender, started) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let mut emotes = TwitchEmotes::spawn_with_fetcher(
            ControlledFetcher {
                calls: Arc::clone(&calls),
                started: started_sender,
                release: release_receiver,
            },
            EventLogger::disabled(),
            || {},
        );
        emotes.set_enabled(true);
        let referenced = std::iter::once("first".to_owned())
            .chain((0..super::REQUEST_CAPACITY - 1).map(|index| format!("queued-{index}")))
            .collect::<Vec<_>>();
        emotes.retain_referenced(referenced.iter().map(String::as_str));
        assert!(emotes.texture("first", Instant::now()).is_none());
        started.recv().expect("worker started");
        for index in 0..super::REQUEST_CAPACITY - 1 {
            assert!(
                emotes
                    .texture(&format!("queued-{index}"), Instant::now())
                    .is_none()
            );
        }

        let shutdown = Arc::clone(&emotes.shutdown);
        let dropper = thread::spawn(move || drop(emotes));
        let deadline = Instant::now() + Duration::from_secs(1);
        while !shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "drop did not request shutdown");
            thread::yield_now();
        }
        release.send(()).expect("release fetch");
        dropper.join().expect("drop worker");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn disabling_discards_stale_results_and_skips_queued_network_work() {
        struct ControlledFetcher {
            calls: Arc<AtomicUsize>,
            started: mpsc::SyncSender<String>,
            release: mpsc::Receiver<()>,
        }

        impl EmoteFetcher for ControlledFetcher {
            fn fetch(&mut self, id: &str) -> Result<DecodedEmote, EmoteError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.started
                    .send(id.to_owned())
                    .expect("publish fetch identity");
                self.release.recv().expect("release fetch");
                Ok(DecodedEmote {
                    size: [1, 1],
                    rgba: vec![255, 255, 255, 255],
                })
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let (started_sender, started) = mpsc::sync_channel(2);
        let (release, release_receiver) = mpsc::sync_channel(2);
        let mut emotes = TwitchEmotes::spawn_with_fetcher(
            ControlledFetcher {
                calls: Arc::clone(&calls),
                started: started_sender,
                release: release_receiver,
            },
            EventLogger::disabled(),
            || {},
        );
        emotes.set_enabled(true);
        let referenced = std::iter::once("first".to_owned())
            .chain((0..super::REQUEST_CAPACITY - 1).map(|index| format!("stale-{index}")))
            .collect::<Vec<_>>();
        emotes.retain_referenced(referenced.iter().map(String::as_str));
        assert!(emotes.texture("first", Instant::now()).is_none());
        assert_eq!(started.recv().expect("first fetch"), "first");
        for index in 0..super::REQUEST_CAPACITY - 1 {
            assert!(
                emotes
                    .texture(&format!("stale-{index}"), Instant::now())
                    .is_none()
            );
        }

        emotes.set_enabled(false);
        release.send(()).expect("release stale fetch");
        emotes.set_enabled(true);
        emotes.retain_referenced(["fresh"].into_iter());
        assert!(emotes.texture("fresh", Instant::now()).is_none());
        assert_eq!(
            started
                .recv_timeout(Duration::from_secs(1))
                .expect("fresh request"),
            "fresh"
        );
        release.send(()).expect("release fresh fetch");

        let context = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            emotes.poll(&context, Instant::now());
            if emotes.texture("fresh", Instant::now()).is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "fresh result timed out");
            thread::yield_now();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn draining_only_stale_requests_requests_one_retry_frame() {
        struct ControlledFetcher {
            started: mpsc::SyncSender<()>,
            release: mpsc::Receiver<()>,
        }

        impl EmoteFetcher for ControlledFetcher {
            fn fetch(&mut self, _id: &str) -> Result<DecodedEmote, EmoteError> {
                self.started.send(()).expect("publish fetch start");
                self.release.recv().expect("release fetch");
                Err(EmoteError::Transport)
            }
        }

        let (started_sender, started) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let repaints = Arc::new(AtomicUsize::new(0));
        let repaint_counter = Arc::clone(&repaints);
        let mut emotes = TwitchEmotes::spawn_with_fetcher(
            ControlledFetcher {
                started: started_sender,
                release: release_receiver,
            },
            EventLogger::disabled(),
            move || {
                repaint_counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        emotes.set_enabled(true);
        emotes.retain_referenced(["first", "queued"].into_iter());
        assert!(emotes.texture("first", Instant::now()).is_none());
        started.recv().expect("first fetch");
        assert!(emotes.texture("queued", Instant::now()).is_none());

        emotes.reset();
        release.send(()).expect("release stale fetch");
        let deadline = Instant::now() + Duration::from_secs(1);
        while repaints.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "retry repaint timed out");
            thread::yield_now();
        }
        assert_eq!(repaints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn one_post_reset_stale_result_cannot_displace_any_current_result() {
        let mut emotes = TwitchEmotes::disabled();
        emotes.set_enabled(true);
        let generation = emotes.generation.load(Ordering::SeqCst);
        let ids = (0..super::REQUEST_CAPACITY)
            .map(|index| format!("fresh-{index}"))
            .collect::<Vec<_>>();
        emotes.retain_referenced(ids.iter().map(String::as_str));
        emotes.pending.extend(ids.iter().cloned());

        let (sender, results) = mpsc::sync_channel(super::RESULT_CAPACITY);
        sender
            .try_send(super::FetchResult {
                generation: generation.saturating_sub(1),
                id: "stale".to_owned(),
                result: Ok(one_pixel_emote()),
            })
            .expect("reserved stale result slot");
        for id in &ids {
            sender
                .try_send(super::FetchResult {
                    generation,
                    id: id.clone(),
                    result: Ok(one_pixel_emote()),
                })
                .expect("current result capacity");
        }
        drop(sender);
        emotes.results = results;
        emotes.poll(&egui::Context::default(), Instant::now());

        assert!(emotes.pending.is_empty());
        assert_eq!(emotes.entries.len(), super::REQUEST_CAPACITY);
    }

    fn one_pixel_emote() -> DecodedEmote {
        DecodedEmote {
            size: [1, 1],
            rgba: vec![255, 255, 255, 255],
        }
    }

    fn encode_png(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::new(width, height));
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode test PNG");
        bytes.into_inner()
    }
}
