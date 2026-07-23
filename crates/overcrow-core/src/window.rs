use overcrow_protocol::{GameWindow, Rect};

use crate::ProcessClassification;

#[derive(Clone, Debug, PartialEq)]
pub struct WindowObservation {
    pub pid: Option<u32>,
    pub app_id: Option<String>,
    pub title: String,
    pub rect: Rect,
    pub scale: f64,
    pub backend: String,
}

impl WindowObservation {
    pub fn into_game(self, classification: ProcessClassification) -> GameWindow {
        GameWindow {
            pid: self.pid,
            steam_app_id: classification.steam_app_id,
            app_id: self.app_id,
            title: self.title,
            rect: self.rect,
            scale: self.scale,
            backend: self.backend,
        }
    }
}

pub trait WindowSource {
    fn active_window(&mut self) -> anyhow::Result<Option<WindowObservation>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopWindowSource;

impl WindowSource for NoopWindowSource {
    fn active_window(&mut self) -> anyhow::Result<Option<WindowObservation>> {
        Ok(None)
    }
}
