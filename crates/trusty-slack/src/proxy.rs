//! Proxy mode — Izzie drafts responses on the user's behalf.
//!
//! When proxy mode is active for a channel:
//!   1. Any message in that channel triggers Izzie to draft a reply
//!   2. The draft is DMed to the primary user with context
//!   3. The user replies "send [optional edits]" → Izzie posts it to the channel
//!   4. Any other reply text → Izzie posts that text instead
//!
//! Configuration (stored in kv_config):
//!   `slack_proxy_channels`  — comma-separated channel IDs to monitor
//!   `slack_primary_user_id` — Slack user ID (U...) to DM drafts to
//!   `slack_primary_dm_id`   — DM channel ID (D...) to the primary user
//!
//! Pending proxy draft state is kept in-memory:
//!   dm_thread_ts → PendingDraft { original_channel, original_thread_ts, draft }

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;
use tracing::{error, info};
use uuid::Uuid;

use trusty_chat::ChatEngine;
use trusty_models::chat::ChatSession;
use trusty_store::Store;

use crate::api;

/// A draft queued for the user's approval.
#[derive(Debug, Clone)]
pub struct PendingDraft {
    /// Channel the original message came from.
    pub original_channel: String,
    /// Thread timestamp to reply into.
    pub original_thread_ts: String,
    /// The draft text Izzie generated.
    pub draft: String,
}

/// Shared proxy state.
#[derive(Default)]
pub struct ProxyState {
    /// dm_thread_ts → pending draft
    pub pending: Mutex<HashMap<String, PendingDraft>>,
}

impl ProxyState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }
}

/// Check if `channel` is in the proxy channel list (stored in kv_config).
pub async fn is_proxy_channel(store: &Store, channel: &str) -> bool {
    let raw = store
        .sqlite
        .get_config("slack_proxy_channels")
        .unwrap_or_default()
        .unwrap_or_default();
    raw.split(',').map(|s| s.trim()).any(|c| c == channel)
}

/// Return the DM channel ID for the primary Slack user, if configured.
pub async fn primary_dm_channel(store: &Store) -> Option<String> {
    store
        .sqlite
        .get_config("slack_primary_dm_id")
        .unwrap_or_default()
}

/// Draft a proxy reply and DM it to the primary user for approval.
///
/// Returns the dm_thread_ts of the DM message so we can track the approval.
#[allow(clippy::too_many_arguments)]
pub async fn send_draft_for_approval(
    engine: &ChatEngine,
    store: &Arc<Store>,
    bot_token: &str,
    proxy_state: &ProxyState,
    sessions: &Mutex<HashMap<String, ChatSession>>,
    original_channel: &str,
    original_thread_ts: &str,
    sender_display: &str,
    message_text: &str,
) {
    let dm_channel = match primary_dm_channel(store).await {
        Some(c) => c,
        None => {
            info!("Proxy mode: no slack_primary_dm_id configured — skipping draft");
            return;
        }
    };

    // Draft using ChatEngine.
    let instance_id = std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| "slack".into());
    let draft_session_key = format!(
        "slack-proxy-draft:{}:{}",
        original_channel, original_thread_ts
    );
    let prompt = format!(
        "You are helping draft a Slack reply on the user's behalf.\n\
        Channel: {original_channel}\n\
        Message from {sender_display}:\n\
        {message_text}\n\n\
        Write a concise, professional reply the user can send. \
        Match the user's typical communication style. \
        Return only the reply text — no preamble, no explanation."
    );

    let mut sessions_guard = sessions.lock().await;
    let session = sessions_guard
        .entry(draft_session_key.clone())
        .or_insert_with(|| {
            let now = Utc::now();
            ChatSession {
                id: Uuid::new_v4(),
                user_id: instance_id.clone(),
                title: Some(format!(
                    "Proxy draft {original_channel}/{original_thread_ts}"
                )),
                messages: vec![],
                is_compressed: false,
                created_at: now,
                updated_at: now,
            }
        });

    let draft_result = engine.chat(session, &prompt).await;
    drop(sessions_guard);

    let draft = match draft_result {
        Ok(r) => r.reply,
        Err(e) => {
            error!("Failed to draft proxy reply: {e}");
            return;
        }
    };

    // DM the user with the draft for approval.
    let dm_text = format!(
        "💬 *New message in <#{original_channel}> from {sender_display}:*\n\
        _{message_text}_\n\n\
        *Suggested reply:*\n\
        {draft}\n\n\
        Reply *send* to post this, reply with your own text to send that instead, \
        or *ignore* to skip."
    );

    match api::post_message(bot_token, &dm_channel, None, &dm_text).await {
        Ok(()) => {
            info!("Proxy draft DMed to primary user");
            // Store the pending draft keyed by the DM's message ts.
            // We don't get the ts back from post_message without parsing the full response.
            // Simpler: key by (original_channel + original_thread_ts) and look up when
            // the user replies in the DM channel.
            let pending_key = format!("{}:{}", original_channel, original_thread_ts);
            proxy_state.pending.lock().await.insert(
                pending_key,
                PendingDraft {
                    original_channel: original_channel.to_string(),
                    original_thread_ts: original_thread_ts.to_string(),
                    draft: draft.clone(),
                },
            );
        }
        Err(e) => error!("Failed to DM proxy draft: {e}"),
    }
}

/// Handle a user reply in their DM channel — check if it's a proxy approval.
///
/// Returns true if this message was handled as a proxy approval (caller should
/// not also handle it as a regular chat message).
pub async fn handle_possible_approval(
    bot_token: &str,
    proxy_state: &ProxyState,
    _dm_channel: &str,
    user_text: &str,
) -> bool {
    let mut pending = proxy_state.pending.lock().await;
    if pending.is_empty() {
        return false;
    }

    let lower = user_text.trim().to_lowercase();

    // Find the most recent pending draft (there may be several).
    // For simplicity we take the first one queued.
    let key = match pending.keys().next().cloned() {
        Some(k) => k,
        None => return false,
    };
    let draft_info = pending.remove(&key).unwrap();

    if lower == "ignore" || lower == "skip" {
        info!("Proxy draft for {} ignored by user", key);
        return true;
    }

    // Determine what to post.
    let text_to_send = if lower == "send" || lower == "yes" || lower == "ok" {
        draft_info.draft.clone()
    } else {
        // User provided their own reply text.
        user_text.trim().to_string()
    };

    info!(
        "Posting proxy reply to channel {} thread {}",
        draft_info.original_channel, draft_info.original_thread_ts
    );

    if let Err(e) = api::post_message(
        bot_token,
        &draft_info.original_channel,
        Some(&draft_info.original_thread_ts),
        &text_to_send,
    )
    .await
    {
        error!("Failed to post proxy reply: {e}");
    }

    true
}

/// Add a channel to the proxy watch list.
pub fn add_proxy_channel(store: &Store, channel_id: &str) -> anyhow::Result<()> {
    let current = store
        .sqlite
        .get_config("slack_proxy_channels")
        .unwrap_or_default()
        .unwrap_or_default();
    let mut channels: Vec<&str> = current
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if !channels.contains(&channel_id) {
        channels.push(channel_id);
    }
    store
        .sqlite
        .set_config("slack_proxy_channels", &channels.join(","))?;
    Ok(())
}

/// Remove a channel from the proxy watch list.
pub fn remove_proxy_channel(store: &Store, channel_id: &str) -> anyhow::Result<()> {
    let current = store
        .sqlite
        .get_config("slack_proxy_channels")
        .unwrap_or_default()
        .unwrap_or_default();
    let channels: Vec<&str> = current
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != channel_id)
        .collect();
    store
        .sqlite
        .set_config("slack_proxy_channels", &channels.join(","))?;
    Ok(())
}
