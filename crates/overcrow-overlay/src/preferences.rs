use std::io;

#[cfg(test)]
use std::path::PathBuf;

use overcrow_config::{WidgetSettingsLoad, WidgetSettingsStore};

pub type OverlayPreferences = overcrow_config::WidgetProfile;

pub struct PreferenceStore {
    inner: WidgetSettingsStore,
}

impl PreferenceStore {
    pub fn from_environment() -> Self {
        Self {
            inner: WidgetSettingsStore::from_environment(),
        }
    }

    #[cfg(test)]
    fn from_paths(path: impl Into<PathBuf>, legacy_path: impl Into<PathBuf>) -> Self {
        Self {
            inner: WidgetSettingsStore::from_paths(path, legacy_path),
        }
    }

    pub fn load(&self) -> WidgetSettingsLoad {
        self.inner.load()
    }

    pub fn save(&self, preferences: &OverlayPreferences) -> io::Result<()> {
        self.inner.save(preferences)
    }
}

#[cfg(test)]
#[path = "preferences_tests.rs"]
mod tests;
