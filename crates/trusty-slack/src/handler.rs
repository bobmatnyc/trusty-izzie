//! Axum handler for POST /slack/events.
//!
//! First-class chat (like Telegram) + proxy mode:
//!
//! CHAT mode (DMs + @mentions):
//!   • Maintains per-thread ChatSession with full tool access
//!   • User can invoke all Izzie tools (weather, trains, web search, etc.)
//!   • Command: "watch #channel-id" — add channel to proxy watch list
//!   • Command: "unwatch #channel-id" — remove from proxy watch list
//!   • Command: "set my slack id U123" — store user's Slack ID for DM routing
//!
//! PROXY mode (channel monitoring):
//!   • Izzie watches configured channels for any messages
//!   • Drafts a reply and DMs it to the primary user for approval
//!   • User replies "send" to post the draft, or provides their own text
//!   • User replies "ignore" / "skip" to drop the draft

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use uuid::Uuid;

use trusty_chat::ChatEngine;
use trusty_models::chat::ChatSession;
use trusty_store::Store;

use crate::{
    api,
    events::{ChallengeResponse, EventCallback, MessageEvent, SlackEvent, SlackPayload},
    proxy::{self, ProxyState},
    verify,
};

/// Shared application state injected into every handler.
pub struct SlackState {
    pub engine: Arc<ChatEngine>,
    pub store: Arc<Store>,
    pub bot_token: String,
    /// Optional user token (xoxp-...) — when set, approved proxy posts are sent
    /// as the user rather than as the bot.
    pub user_token: Option<String>,
    pub signing_secret: String,
    /// Per-thread conversation sessions ("slack:channel:thread_ts" → ChatSession).
    pub sessions: Arc<Mutex<HashMap<String, ChatSession>>>,
    /// Proxy mode state (pending approval drafts).
    pub proxy: Arc<ProxyState>,
}

/// POST /slack/events — main webhook entry point.
pub async fn handle_event(State(state): State<Arc<SlackState>>, req: Request) -> impl IntoResponse {
    let timestamp = req
        .headers()
        .get("X-Slack-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let signature = req
        .headers()
        .get("X-Slack-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            error!("Failed to read body: {e}");
            return (StatusCode::BAD_REQUEST, "body read error").into_response();
        }
    };

    if let Err(e) = verify::verify(&state.signing_secret, &timestamp, &signature, &body_bytes) {
        warn!("Signature verification failed: {e}");
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    let payload: SlackPayload = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to parse payload: {e}");
            return (StatusCode::BAD_REQUEST, "parse error").into_response();
        }
    };

    match payload {
        SlackPayload::UrlVerification { challenge } => {
            info!("Slack URL verification handshake");
            Json(ChallengeResponse { challenge }).into_response()
        }
        SlackPayload::EventCallback(callback) => {
            // Spawn so we always return 200 to Slack within 3 s.
            tokio::spawn(dispatch_event(state, *callback));
            StatusCode::OK.into_response()
        }
        SlackPayload::Unknown => StatusCode::OK.into_response(),
    }
}

async fn dispatch_event(state: Arc<SlackState>, callback: EventCallback) {
    match callback.event {
        // @mention in any channel — always respond as Izzie.
        SlackEvent::AppMention(msg) => {
            handle_chat_message(&state, msg, true).await;
        }
        SlackEvent::Message(msg) => {
            // Ignore bot messages and edited/deleted subtypes.
            if msg.bot_id.is_some() || msg.subtype.is_some() {
                return;
            }

            let is_dm = msg
                .channel_type
                .as_deref()
                .map(|t| t == "im")
                .unwrap_or(false);

            if is_dm {
                // DM: check if the user is approving a proxy draft first.
                let user_text = msg.text.as_deref().unwrap_or("").trim().to_string();
                if !user_text.is_empty() {
                    let handled = proxy::handle_possible_approval(
                        &state.bot_token,
                        state.user_token.as_deref(),
                        &state.proxy,
                        &msg.channel,
                        &user_text,
                    )
                    .await;
                    if !handled {
                        // Regular DM chat with Izzie.
                        handle_chat_message(&state, msg, false).await;
                    }
                }
            } else {
                // Channel message — check if proxy mode is active for this channel.
                if proxy::is_proxy_channel(&state.store, &msg.channel).await {
                    handle_proxy_channel_message(&state, msg).await;
                }
                // If not proxy and not mentioned, ignore (reduce noise).
            }
        }
        SlackEvent::Other => {}
    }
}

/// Full chat with Izzie (DMs and @mentions).
async fn handle_chat_message(state: &Arc<SlackState>, msg: MessageEvent, strip_mention: bool) {
    let channel = &msg.channel;
    let thread_ts = msg.thread_ts.as_deref().unwrap_or(&msg.ts);
    let session_key = format!("slack:{}:{}", channel, thread_ts);

    // Fetch thread history for context.
    let history = api::get_thread_messages(&state.bot_token, channel, thread_ts, 12)
        .await
        .unwrap_or_default();

    // Build user text (strip @mention prefix for app_mention events).
    let raw_text = msg.text.as_deref().unwrap_or("").to_string();
    let user_text = if strip_mention {
        strip_bot_mention(&raw_text)
    } else {
        raw_text.clone()
    };

    if user_text.trim().is_empty() {
        return;
    }

    // Handle config commands in DMs.
    if !strip_mention {
        if let Some(reply) = handle_config_command(state, &user_text).await {
            let _ = api::post_message(&state.bot_token, channel, Some(thread_ts), &reply).await;
            return;
        }
    }

    // Prepend recent thread messages as context.
    let context_lines: Vec<String> = history
        .iter()
        .filter(|m| m.ts != msg.ts)
        .take(10)
        .map(|m| {
            let who = if m.bot_id.is_some() {
                "Izzie".to_string()
            } else {
                m.user.as_deref().unwrap_or("User").to_string()
            };
            format!("{who}: {}", m.text)
        })
        .collect();

    let full_prompt = if context_lines.is_empty() {
        user_text.clone()
    } else {
        format!(
            "[Slack thread — {} prior messages]\n{}\n\n[New message]: {}",
            context_lines.len(),
            context_lines.join("\n"),
            user_text
        )
    };

    info!(session = %session_key, user = ?msg.user, "Slack → ChatEngine");

    let instance_id = std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| "slack".into());
    let mut sessions = state.sessions.lock().await;
    let session = sessions.entry(session_key.clone()).or_insert_with(|| {
        let now = Utc::now();
        ChatSession {
            id: Uuid::new_v4(),
            user_id: instance_id.clone(),
            title: Some(format!("Slack {channel}/{thread_ts}")),
            messages: vec![],
            is_compressed: false,
            created_at: now,
            updated_at: now,
        }
    });

    let result = state.engine.chat(session, &full_prompt).await;
    drop(sessions);

    let reply_text = match result {
        Ok(s) => s.reply,
        Err(e) => {
            error!("ChatEngine error for {session_key}: {e}");
            format!("Sorry, I hit an error: {e}")
        }
    };

    let slack_text = truncate_for_slack(&reply_text);
    if let Err(e) = api::post_message(&state.bot_token, channel, Some(thread_ts), &slack_text).await
    {
        error!("Failed to post reply: {e}");
    }
}

/// Proxy mode: draft a response to a channel message and DM it to the user.
async fn handle_proxy_channel_message(state: &Arc<SlackState>, msg: MessageEvent) {
    let channel = &msg.channel;
    let thread_ts = msg.thread_ts.as_deref().unwrap_or(&msg.ts);
    let sender = msg.user.as_deref().unwrap_or("someone");
    let message_text = msg.text.as_deref().unwrap_or("").trim().to_string();

    if message_text.is_empty() {
        return;
    }

    info!(
        channel = %channel,
        sender = %sender,
        "Proxy mode: generating draft for channel message"
    );

    proxy::send_draft_for_approval(
        &state.engine,
        &state.store,
        &state.bot_token,
        &state.proxy,
        &state.sessions,
        channel,
        thread_ts,
        sender,
        &message_text,
    )
    .await;
}

/// Handle DM config commands. Returns Some(reply) if handled, None if it's a
/// regular chat message.
async fn handle_config_command(state: &Arc<SlackState>, text: &str) -> Option<String> {
    let lower = text.trim().to_lowercase();

    // "watch #C1234ABCD" or "watch C1234ABCD" — add proxy channel
    if let Some(rest) = lower.strip_prefix("watch ") {
        let channel_id = rest.trim().trim_start_matches('#').to_string();
        // Resolve channel name to ID if needed (simple: accept raw IDs only).
        if let Err(e) = proxy::add_proxy_channel(&state.store, &channel_id) {
            return Some(format!("Failed to add proxy channel: {e}"));
        }
        return Some(format!(
            "✅ Now watching <#{channel_id}> for proxy drafts.\n\
            When anyone posts there, I'll DM you a suggested reply for approval."
        ));
    }

    // "unwatch #C1234ABCD"
    if let Some(rest) = lower.strip_prefix("unwatch ") {
        let channel_id = rest.trim().trim_start_matches('#').to_string();
        if let Err(e) = proxy::remove_proxy_channel(&state.store, &channel_id) {
            return Some(format!("Failed to remove proxy channel: {e}"));
        }
        return Some(format!("✅ No longer watching <#{channel_id}>."));
    }

    // "set proxy dm D1234ABCD" — set the DM channel for draft delivery
    if let Some(rest) = lower.strip_prefix("set proxy dm ") {
        let dm_id = rest.trim().to_string();
        if let Err(e) = state.store.sqlite.set_config("slack_primary_dm_id", &dm_id) {
            return Some(format!("Failed to save DM channel: {e}"));
        }
        return Some(format!(
            "✅ Proxy drafts will be sent to DM channel {dm_id}."
        ));
    }

    // "proxy status"
    if lower == "proxy status" || lower == "proxy" {
        let channels = state
            .store
            .sqlite
            .get_config("slack_proxy_channels")
            .unwrap_or_default()
            .unwrap_or_default();
        let dm = state
            .store
            .sqlite
            .get_config("slack_primary_dm_id")
            .unwrap_or_default()
            .unwrap_or_default();
        let pending = state.proxy.pending.lock().await.len();
        return Some(format!(
            "*Proxy Mode Status*\n\
            Watched channels: {}\n\
            DM channel: {}\n\
            Pending drafts: {}",
            if channels.is_empty() {
                "none".into()
            } else {
                channels
            },
            if dm.is_empty() { "not set".into() } else { dm },
            pending
        ));
    }

    None
}

/// Strip `<@UXXXXXXXX>` mention prefix from message text.
fn strip_bot_mention(text: &str) -> String {
    let mut t = text.trim().to_string();
    while t.starts_with('<') {
        if let Some(end) = t.find('>') {
            t = t[end + 1..].trim_start().to_string();
        } else {
            break;
        }
    }
    t
}

/// Cap at ~3,800 chars to stay well within Slack limits.
fn truncate_for_slack(text: &str) -> String {
    const MAX: usize = 3_800;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let truncated: String = text.chars().take(MAX).collect();
    format!(
        "{}\n\n_[Response truncated — ask me to continue]_",
        truncated.trim_end()
    )
}
