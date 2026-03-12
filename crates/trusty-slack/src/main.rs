//! trusty-slack — Slack bot interface for trusty-izzie.
//!
//! A first-class chat interface alongside Telegram. Supports:
//!   • Full ChatEngine chat via @mentions and DMs
//!   • Proxy mode: Izzie monitors channels, drafts replies for the user's approval,
//!     and posts them on the user's behalf
//!
//! Required env vars:
//!   SLACK_BOT_TOKEN      — xoxb-... bot token for Slack Web API
//!   SLACK_SIGNING_SECRET — HMAC-SHA256 webhook signing secret
//!   OPENROUTER_API_KEY   — forwarded to ChatEngine
//!
//! Optional:
//!   SLACK_PORT           — HTTP listen port (default 3457)
//!   TRUSTY_DATA_DIR      — override data directory

mod api;
mod events;
mod handler;
mod proxy;
mod verify;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use axum::{routing::get, routing::post, Router};
use clap::Parser;
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::EnvFilter;

use trusty_chat::ChatEngine;
use trusty_metro_north::MetroNorthSkill;
use trusty_models::chat::ChatSession;
use trusty_store::Store;
use trusty_weather::WeatherSkill;

use crate::handler::SlackState;
use crate::proxy::ProxyState;

/// Slack bot binary for trusty-izzie.
#[derive(Parser)]
#[command(name = "trusty-slack", about = "Izzie Slack bot")]
struct Args {
    /// Optional config file path.
    #[arg(long)]
    config: Option<String>,

    /// Port to listen on (overrides SLACK_PORT, default 3457).
    #[arg(long, env = "SLACK_PORT", default_value = "3457")]
    port: u16,
}

/// Derive instance ID from the primary email (mirrors daemon/telegram approach).
fn load_instance_id() -> String {
    use sha2::{Digest, Sha256};
    let data_dir = std::env::var("TRUSTY_DATA_DIR")
        .unwrap_or_else(|_| "~/.local/share/trusty-izzie".to_string());
    let expanded = shellexpand::tilde(&data_dir).into_owned();
    let path = Path::new(&expanded).join("instance.json");
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
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let signing_secret =
        std::env::var("SLACK_SIGNING_SECRET").expect("SLACK_SIGNING_SECRET must be set");
    let bot_token = std::env::var("SLACK_BOT_TOKEN").expect("SLACK_BOT_TOKEN must be set");
    let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();

    info!("Loading config...");
    let config = trusty_core::load_config(args.config.as_deref()).await?;

    let data_dir_str = shellexpand::tilde(&config.storage.data_dir).into_owned();
    let data_dir = Path::new(&data_dir_str).to_path_buf();
    let instance_id = load_instance_id();

    info!(
        "Opening store at {} (instance {})",
        data_dir.display(),
        instance_id
    );
    let store = Arc::new(Store::open_lazy_kuzu(&data_dir, &instance_id).await?);

    // Build ChatEngine — mirrors trusty-telegram setup.
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

    // In-memory session map: "slack:channel:thread_ts" → ChatSession
    let sessions: Arc<Mutex<HashMap<String, ChatSession>>> = Arc::new(Mutex::new(HashMap::new()));

    // Proxy mode state — pending approval drafts
    let proxy = Arc::new(ProxyState::new());

    let user_token = std::env::var("SLACK_USER_TOKEN").ok();

    let state = Arc::new(SlackState {
        engine,
        store,
        bot_token,
        user_token,
        signing_secret,
        sessions,
        proxy,
    });

    let app = Router::new()
        .route("/slack/events", post(handler::handle_event))
        .route("/health", get(|| async { "ok" }))
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = format!("0.0.0.0:{}", args.port);
    info!("trusty-slack listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
