//! Top-level client configuration, construction, and lifecycle.

mod config;
mod runtime;

#[cfg(test)]
mod tests;

pub use config::{ClientBuilder, ClientConfig, Network};
pub use runtime::SogniClient;
