//! Clipboard helper for Wayland-friendly copy from the overlay.

use eframe::egui;

use super::sanitize::sanitize_clipboard_text;

/// Copy text using both egui's output channel and a direct arboard write.
///
/// egui's `copy_text` alone is unreliable for some Wayland compositors when the
/// overlay is a transparent/passthrough-capable surface. arboard talks to the
/// clipboard protocols more directly.
pub fn copy_text(ctx: &egui::Context, text: &str) -> Result<(), String> {
    let text = sanitize_clipboard_text(text)?;

    ctx.copy_text(text.clone());

    match arboard::Clipboard::new() {
        Ok(mut clipboard) => clipboard.set_text(text).map_err(|error| error.to_string()),
        Err(_error) => {
            // Fall back to egui-only path; the compositor may still honor it.
            Ok(())
        }
    }
}
