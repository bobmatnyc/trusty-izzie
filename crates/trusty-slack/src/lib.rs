//! `trusty-slack` library entry point.
//!
//! Exposes `build_slack_router()` so `trusty-api` can mount the Slack
//! webhook handler at `/slack/events` without needing a separate binary
//! (and therefore no second ngrok tunnel).

pub mod api;
pub mod events;
pub mod handler;
pub mod proxy;
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
/// Returns `None` if `SLACK_BOT_TOKEN` or `SLACK_SIGNING_SECRET` are not set.
pub fn slack_state_from_env(
    engine: Arc<trusty_chat::ChatEngine>,
    store: Arc<trusty_store::Store>,
) -> Option<Arc<SlackState>> {
    let bot_token = std::env::var("SLACK_BOT_TOKEN").ok()?;
    let signing_secret = std::env::var("SLACK_SIGNING_SECRET").ok()?;

    let user_token = std::env::var("SLACK_USER_TOKEN").ok();

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
