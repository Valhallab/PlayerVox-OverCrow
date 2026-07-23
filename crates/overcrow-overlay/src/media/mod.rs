mod client;
mod model;
mod mpris;

pub use client::MediaClient;
#[allow(unused_imports)]
pub use model::{MediaAction, MediaCapabilities, MediaCommand, MediaPlaybackStatus, MediaSnapshot};

#[cfg(test)]
mod tests;
