//! Shared application state passed to all axum handlers.

use std::path::PathBuf;
use std::sync::Arc;

use trusty_models::config::AppConfig;
use trusty_store::SqliteStore;

/// Shared state available in every axum handler via `axum::extract::State`.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub sqlite: Arc<SqliteStore>,
    pub agents_dir: PathBuf,
}

impl AppState {
    pub fn new(config: AppConfig, sqlite: Arc<SqliteStore>) -> Self {
        let agents_dir = {
            let raw = &config.agents.agents_dir;
            PathBuf::from(raw)
        };
        Self {
            config: Arc::new(config),
            sqlite,
            agents_dir,
        }
    }
}
