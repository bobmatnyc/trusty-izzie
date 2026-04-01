//! trusty-api — REST API server for trusty-izzie.
//!
//! Also mounts /slack/events when SLACK_BOT_TOKEN + SLACK_SIGNING_SECRET
//! are set, so the existing ngrok tunnel (port 3456) handles both the
//! REST API and the Slack webhook — no second tunnel needed.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use trusty_api::{routes::build_router, AppState};
use trusty_chat::ChatEngine;
use trusty_core::{init_logging, load_config};
use trusty_metro_north::MetroNorthSkill;
use trusty_store::{SqliteStore, Store};
use trusty_weather::WeatherSkill;

fn expand_data_dir(raw: &str) -> PathBuf {
    shellexpand::tilde(raw).into_owned().into()
}

fn load_instance_id(data_dir: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let path = data_dir.join("instance.json");
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(id) = val.get("instance_id").and_then(|v| v.as_str()) {
                return id.to_string();
            }
        }
    }
    let email = std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_default();
    let hash = Sha256::digest(email.as_bytes());
    hex::encode(&hash[..8])
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    trusty_core::secrets::migrate_from_env();
    let log_level = std::env::var("TRUSTY_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    init_logging(&log_level);

    let config = load_config(None).await?;
    let bind_addr = format!("{}:{}", config.api.host, config.api.port);
    info!(address = %bind_addr, "starting trusty-api");

    let data_dir = expand_data_dir(&config.storage.data_dir);
    let sqlite_path = data_dir.join(&config.storage.sqlite_path);
    let sqlite = Arc::new(SqliteStore::open(&sqlite_path)?);
    let state = AppState::new(config.clone(), Arc::clone(&sqlite));

    // ── Build the main REST router ────────────────────────────────────────
    let mut app = build_router(state).layer(TraceLayer::new_for_http());

    // ── Optionally mount /slack/events ────────────────────────────────────
    // Active when SLACK_BOT_TOKEN and SLACK_SIGNING_SECRET are both set.
    // Shares this port (3456) so the existing ngrok tunnel works without
    // a second tunnel.
    let has_slack = trusty_core::secrets::get("SLACK_BOT_TOKEN").is_some()
        && trusty_core::secrets::get("SLACK_SIGNING_SECRET").is_some();

    if has_slack {
        let api_key = trusty_core::secrets::get("OPENROUTER_API_KEY").unwrap_or_default();
        let instance_id = load_instance_id(&data_dir);

        match Store::open_lazy_kuzu_read_only(&data_dir, &instance_id).await {
            Ok(store) => {
                let store = Arc::new(store);
                let engine = Arc::new(
                    ChatEngine::new(
                        config.openrouter.base_url.clone(),
                        api_key,
                        config.openrouter.chat_model.clone(),
                        config.chat.max_tool_iterations,
                    )
                    .with_sqlite(Arc::clone(&store.sqlite))
                    .with_agents_dir(data_dir.join("agents"))
                    .with_skills_dir(config.agents.skills_dir.clone())
                    .with_skills(vec![Arc::new(MetroNorthSkill), Arc::new(WeatherSkill)]),
                );

                if let Some(slack_state) = trusty_slack::slack_state_from_env(engine, store) {
                    let slack_router = trusty_slack::build_slack_router(slack_state);
                    app = app.merge(slack_router);
                    info!("Slack webhook mounted at /slack/events");
                }
            }
            Err(e) => {
                warn!("Could not open Store for Slack integration: {e} — Slack disabled");
            }
        }
    } else {
        info!("Slack disabled (SLACK_BOT_TOKEN / SLACK_SIGNING_SECRET not set)");
    }

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
