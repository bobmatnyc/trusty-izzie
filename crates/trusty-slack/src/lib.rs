//! `trusty-slack` library entry point.
//!
//! Two transport modes:
//!
//! **HTTP webhook** (legacy): Mount `build_slack_router()` on axum at `/slack/events`.
//! Requires a public URL (ngrok or real domain). `SLACK_SIGNING_SECRET` required.
//!
//! **Socket Mode** (recommended for ELT rollout): Call `spawn_socket_mode()` from
//! your tokio runtime. No public URL needed — Slack connects outbound via WSS.
//! Requires `SLACK_APP_TOKEN` (xapp-...) in addition to `SLACK_BOT_TOKEN`.

pub mod api;
pub mod events;
pub mod handler;
pub mod proxy;
pub mod socket_mode;
pub mod verify;

use std::collections::HashMap;
use std::sync::Arc;

use axum::{routing::post, Router};
use tokio::sync::Mutex;
use trusty_models::chat::ChatSession;

pub use handler::SlackState;

/// Build an axum sub-router that handles all Slack Events API traffic.
///
/// Mount this on your existing axum app:
/// ```rust
/// let app = existing_router
///     .merge(trusty_slack::build_slack_router(slack_state));
/// ```
pub fn build_slack_router(state: Arc<SlackState>) -> Router {
    Router::new()
        .route("/slack/events", post(handler::handle_event))
        .with_state(state)
}

/// Convenience function: build `SlackState` from environment variables and
/// the provided engine + store.
///
/// Returns `None` if `SLACK_BOT_TOKEN` is not set.
/// `SLACK_SIGNING_SECRET` is optional when using Socket Mode only.
pub fn slack_state_from_env(
    engine: Arc<trusty_chat::ChatEngine>,
    store: Arc<trusty_store::Store>,
) -> Option<Arc<SlackState>> {
    let bot_token = trusty_core::secrets::get("SLACK_BOT_TOKEN")?;
    // signing_secret only required for HTTP webhook mode; empty string disables verification
    let signing_secret = trusty_core::secrets::get("SLACK_SIGNING_SECRET").unwrap_or_default();
    let user_token = trusty_core::secrets::get("SLACK_USER_TOKEN");

    Some(Arc::new(SlackState {
        engine,
        store,
        bot_token,
        user_token,
        signing_secret,
        sessions: Arc::new(Mutex::new(HashMap::<String, ChatSession>::new())),
        proxy: Arc::new(proxy::ProxyState::new()),
    }))
}

/// Launch Slack Socket Mode in a background task.
///
/// When `SLACK_APP_TOKEN` is set, starts a persistent WebSocket connection to
/// Slack that receives all events without requiring a public HTTP endpoint.
/// The task reconnects automatically on disconnect.
///
/// Returns `true` if Socket Mode was started, `false` if `SLACK_APP_TOKEN` is absent.
pub fn spawn_socket_mode(state: Arc<SlackState>) -> bool {
    let app_token = match trusty_core::secrets::get("SLACK_APP_TOKEN").filter(|s| !s.is_empty()) {
        Some(t) => t,
        None => return false,
    };
    tracing::info!("Slack Socket Mode: starting background task");
    tokio::spawn(socket_mode::run(state, app_token));
    true
}
