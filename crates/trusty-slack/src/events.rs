//! Slack Events API payload types.
//!
//! Covers url_verification challenge and event_callback wrappers for:
//!   - app_mention  (bot was @mentioned in a channel)
//!   - message      (DM or channel message — filtered by channel_type)

use serde::{Deserialize, Serialize};

/// Top-level envelope from Slack Events API.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackPayload {
    /// Slack sends this once to verify the endpoint URL.
    UrlVerification { challenge: String },
    /// Wrapper for any event subscription.
    EventCallback(Box<EventCallback>),
    /// Ignored unknown types.
    #[serde(other)]
    Unknown,
}

/// event_callback wrapper.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct EventCallback {
    pub team_id: String,
    pub event: SlackEvent,
    /// Unique ID for this event delivery — used for deduplication.
    pub event_id: String,
}

/// The inner event object.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackEvent {
    /// Bot was @-mentioned in a channel.
    AppMention(MessageEvent),
    /// A message was posted (DMs, channel messages).
    Message(MessageEvent),
    /// Any other event type — ignored.
    #[serde(other)]
    Other,
}

/// Common fields for message-like events.
#[derive(Debug, Deserialize, Clone)]
pub struct MessageEvent {
    /// The channel (or DM channel) the message was posted in.
    pub channel: String,
    /// `im` for DMs, `mpim` for group DMs, `channel` / `group` for channels.
    #[serde(default)]
    pub channel_type: Option<String>,
    /// Slack user ID of the sender.
    pub user: Option<String>,
    /// Bot ID — set when the sender is a bot; we use this to ignore our own messages.
    pub bot_id: Option<String>,
    /// The message text.
    pub text: Option<String>,
    /// Message timestamp (also used as the message ID).
    pub ts: String,
    /// Parent thread timestamp — None for top-level messages.
    pub thread_ts: Option<String>,
    /// Subtypes like `bot_message`, `message_changed`, etc.
    #[serde(default)]
    pub subtype: Option<String>,
}

/// A message item from conversations.replies API.
#[derive(Debug, Deserialize, Clone)]
pub struct ConversationMessage {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    pub text: String,
    pub ts: String,
}

/// Response body for url_verification.
#[derive(Serialize)]
pub struct ChallengeResponse {
    pub challenge: String,
}
