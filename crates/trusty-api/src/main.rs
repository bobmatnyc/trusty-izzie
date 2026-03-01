//! trusty-api — REST API server for trusty-izzie.

use anyhow::Result;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

use trusty_api::{routes::build_router, AppState};
use trusty_core::{init_logging, load_config};

#[tokio::main]
async fn main() -> Result<()> {
    let log_level = std::env::var("TRUSTY_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    init_logging(&log_level);

    let config = load_config(None).await?;

    let bind_addr = format!("{}:{}", config.api.host, config.api.port);
    info!(address = %bind_addr, "starting trusty-api");

    let state = AppState::new(config);

    let app = build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
