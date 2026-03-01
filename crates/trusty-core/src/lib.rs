//! Shared traits, config loading, error types, and logging setup for trusty-izzie.
//!
//! Every binary crate depends on this crate. It provides the bootstrap
//! sequence (load config, init logging) and the canonical `TrustyError` type.

pub mod config;
pub mod error;
pub mod logging;

pub use config::load_config;
pub use error::{Result, TrustyError};
pub use trusty_models::config::AppConfig;

/// Initialize structured logging with `tracing-subscriber`.
///
/// Call this once, early in `main()`, before any async work.
///
/// # Arguments
/// * `level` - A valid `EnvFilter` directive such as `"info"` or `"trusty=debug,warn"`.
pub fn init_logging(level: &str) {
    logging::init(level);
}
