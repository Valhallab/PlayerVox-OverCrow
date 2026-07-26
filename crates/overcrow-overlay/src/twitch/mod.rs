pub mod auth;
pub mod client;
pub mod credentials;
pub mod emotes;
pub mod eventsub;
pub mod http;
pub mod model;
pub mod prefs;

#[cfg(test)]
#[path = "auth_tests.rs"]
mod auth_tests;

#[cfg(test)]
#[path = "eventsub_tests.rs"]
mod eventsub_tests;

#[cfg(test)]
#[path = "http_tests.rs"]
mod http_tests;

#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;
