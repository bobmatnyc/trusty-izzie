//! Logging initialisation using `tracing-subscriber`.

use tracing_subscriber::{fmt, EnvFilter};

/// Initialise the global tracing subscriber.
///
/// Safe to call only once; subsequent calls are silently ignored because
/// `try_init` returns an error instead of panicking.
pub fn init(level: &str) {
    let filter = EnvFilter::try_new(level)
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(false)
        .try_init();
}
