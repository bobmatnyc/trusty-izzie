//! Shared application state passed to all axum handlers.

use std::sync::Arc;

use trusty_models::config::AppConfig;

/// Shared state available in every axum handler via `axum::extract::State`.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}
