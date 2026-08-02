use std::{
    io::Cursor,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eframe::egui;
use image::{DynamicImage, ImageFormat, RgbaImage};

use super::avatars::{
    AvatarError, AvatarFetcher, AvatarKey, DecodedAvatar, DiscordAvatars, avatar_url, decode_png,
};

#[test]
fn avatar_urls_are_exact_and_accept_only_valid_discord_identifiers() {
    assert_eq!(
        avatar_url(&AvatarKey::new("123", "a_hash_42").unwrap()).unwrap(),
        "https://cdn.discordapp.com/avatars/123/a_hash_42.png?size=64"
    );
    assert!(AvatarKey::new("../123", "hash").is_err());
    assert!(AvatarKey::new("123", "hash/escape").is_err());
    assert!(AvatarKey::new("123", "").is_err());
}

#[test]
fn avatar_decoder_rejects_oversized_dimensions() {
    let valid = png(64, 64);
    assert_eq!(decode_png(&valid).unwrap().size, [64, 64]);

    let oversized = png(257, 64);
    assert_eq!(decode_png(&oversized), Err(AvatarError::TooLarge));
}

#[derive(Clone, Default)]
struct FakeFetcher {
    requested: Arc<Mutex<Vec<AvatarKey>>>,
}

impl AvatarFetcher for FakeFetcher {
    fn fetch(&mut self, key: &AvatarKey) -> Result<DecodedAvatar, AvatarError> {
        self.requested.lock().unwrap().push(key.clone());
        Ok(DecodedAvatar {
            size: [1, 1],
            rgba: vec![255, 255, 255, 255],
        })
    }
}

#[test]
fn avatar_requests_are_deduplicated_and_loaded_off_thread() {
    let fetcher = FakeFetcher::default();
    let requested = Arc::clone(&fetcher.requested);
    let mut avatars = DiscordAvatars::spawn_with_fetcher(fetcher, || {});
    let key = AvatarKey::new("123", "hash").unwrap();
    avatars.set_enabled(true);
    avatars.retain_referenced(std::iter::once(&key));

    let now = Instant::now();
    assert!(avatars.texture(&key, now).is_none());
    assert!(avatars.texture(&key, now).is_none());
    let context = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        avatars.poll(&context, now);
        if avatars.texture(&key, now).is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "avatar fetch did not complete");
        std::thread::yield_now();
    }

    assert_eq!(
        requested.lock().unwrap().as_slice(),
        std::slice::from_ref(&key)
    );
}

#[test]
fn disabling_the_pipeline_drops_cached_and_stale_results() {
    let fetcher = FakeFetcher::default();
    let mut avatars = DiscordAvatars::spawn_with_fetcher(fetcher, || {});
    let key = AvatarKey::new("123", "hash").unwrap();
    avatars.set_enabled(true);
    avatars.retain_referenced(std::iter::once(&key));
    assert!(avatars.texture(&key, Instant::now()).is_none());

    avatars.set_enabled(false);
    avatars.poll(&egui::Context::default(), Instant::now());

    assert!(avatars.texture(&key, Instant::now()).is_none());
    assert_eq!(avatars.cached_len(), 0);
}

#[test]
fn suspension_retains_ready_avatars_without_fetching_while_disabled() {
    let fetcher = FakeFetcher::default();
    let requested = Arc::clone(&fetcher.requested);
    let mut avatars = DiscordAvatars::spawn_with_fetcher(fetcher, || {});
    let key = AvatarKey::new("123", "hash").unwrap();
    avatars.set_enabled(true);
    avatars.retain_referenced(std::iter::once(&key));

    let context = egui::Context::default();
    let now = Instant::now();
    let deadline = now + Duration::from_secs(1);
    while avatars.texture(&key, now).is_none() {
        avatars.poll(&context, now);
        assert!(Instant::now() < deadline, "avatar fetch did not complete");
        std::thread::yield_now();
    }
    assert_eq!(requested.lock().unwrap().len(), 1);

    avatars.set_enabled(false);
    assert!(avatars.texture(&key, now).is_none());
    assert_eq!(requested.lock().unwrap().len(), 1);

    avatars.set_enabled(true);
    assert!(
        avatars.texture(&key, now).is_some(),
        "refocus should reuse the ready in-memory texture"
    );
    assert_eq!(requested.lock().unwrap().len(), 1);

    avatars.retain_referenced(std::iter::empty());
    assert_eq!(avatars.cached_len(), 0);
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = DynamicImage::ImageRgba8(RgbaImage::new(width, height));
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("encode test PNG");
    bytes.into_inner()
}
