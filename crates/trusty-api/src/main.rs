//! trusty-api — REST API server for trusty-izzie.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

use trusty_api::{routes::build_router, AppState};
use trusty_core::{init_logging, load_config};
use trusty_store::SqliteStore;

fn expand_data_dir(raw: &str) -> PathBuf {
    if raw.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(raw.replacen('~', &home, 1))
    } else {
        PathBuf::from(raw)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_level = std::env::var("TRUSTY_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    init_logging(&log_level);

    let config = load_config(None).await?;

    let bind_addr = format!("{}:{}", config.api.host, config.api.port);
    info!(address = %bind_addr, "starting trusty-api");

    let data_dir = expand_data_dir(&config.storage.data_dir);
    let sqlite_path = data_dir.join(&config.storage.sqlite_path);
    let sqlite = Arc::new(SqliteStore::open(&sqlite_path)?);

    let state = AppState::new(config, sqlite);

    let app = build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
