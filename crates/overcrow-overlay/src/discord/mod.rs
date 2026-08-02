pub mod auth;
pub mod avatars;
pub mod client;
pub mod credentials;
pub mod model;
pub mod oauth;
pub mod rpc;

#[cfg(test)]
#[path = "auth_tests.rs"]
mod auth_tests;

#[cfg(test)]
#[path = "avatar_tests.rs"]
mod avatar_tests;

#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod rpc_tests;
