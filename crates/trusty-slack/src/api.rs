//! Slack Web API client.
//!
//! Covers the operations trusty-slack needs:
//!   - `chat.postMessage`      — post a reply to a channel/thread
//!   - `conversations.replies` — fetch thread history for context
//!   - `users.info`            — resolve a user ID to a display name

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::events::ConversationMessage;

const SLACK_API: &str = "https://slack.com/api";

/// Post a message to a channel, optionally inside a thread.
pub async fn post_message(
    token: &str,
    channel: &str,
    thread_ts: Option<&str>,
    text: &str,
) -> Result<()> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({
        "channel": channel,
        "text": text,
    });
    if let Some(ts) = thread_ts {
        body["thread_ts"] = serde_json::Value::String(ts.to_string());
    }

    let resp: SlackResponse = client
        .post(format!("{SLACK_API}/chat.postMessage"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .context("Slack postMessage request failed")?
        .json()
        .await
        .context("Slack postMessage response parse failed")?;

    if !resp.ok {
        anyhow::bail!(
            "Slack postMessage error: {}",
            resp.error.unwrap_or_default()
        );
    }
    Ok(())
}

/// Fetch the last `limit` messages in a thread.
pub async fn get_thread_messages(
    token: &str,
    channel: &str,
    thread_ts: &str,
    limit: u32,
) -> Result<Vec<ConversationMessage>> {
    let client = reqwest::Client::new();
    let resp: RepliesResponse = client
        .get(format!("{SLACK_API}/conversations.replies"))
        .bearer_auth(token)
        .query(&[
            ("channel", channel),
            ("ts", thread_ts),
            ("limit", &limit.to_string()),
        ])
        .send()
        .await
        .context("Slack conversations.replies failed")?
        .json()
        .await
        .context("Slack replies parse failed")?;

    if !resp.ok {
        anyhow::bail!("Slack replies error: {}", resp.error.unwrap_or_default());
    }
    Ok(resp.messages.unwrap_or_default())
}

/// Return the display name for a Slack user ID, or the raw ID on error.
#[allow(dead_code)]
pub async fn get_user_display_name(token: &str, user_id: &str) -> String {
    let client = reqwest::Client::new();
    let result = async {
        let resp: serde_json::Value = client
            .get(format!("{SLACK_API}/users.info"))
            .bearer_auth(token)
            .query(&[("user", user_id)])
            .send()
            .await?
            .json()
            .await?;
        Ok::<serde_json::Value, reqwest::Error>(resp)
    }
    .await;

    match result {
        Ok(resp) => resp["user"]["profile"]["display_name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| resp["user"]["name"].as_str())
            .unwrap_or(user_id)
            .to_string(),
        Err(_) => user_id.to_string(),
    }
}

/// A single suggested prompt shown in the Slack Assistant panel.
pub struct SuggestedPrompt {
    pub title: String,
    pub message: String,
}

/// POST assistant.threads.setSuggestedPrompts
pub async fn set_suggested_prompts(
    token: &str,
    channel_id: &str,
    thread_ts: &str,
    prompts: &[SuggestedPrompt],
) -> Result<()> {
    let prompt_values: Vec<serde_json::Value> = prompts
        .iter()
        .map(|p| serde_json::json!({"title": p.title, "message": p.message}))
        .collect();

    let body = serde_json::json!({
        "channel_id": channel_id,
        "thread_ts": thread_ts,
        "prompts": prompt_values,
    });

    let resp: SlackResponse = reqwest::Client::new()
        .post(format!("{SLACK_API}/assistant.threads.setSuggestedPrompts"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .context("setSuggestedPrompts request failed")?
        .json()
        .await
        .context("setSuggestedPrompts response parse failed")?;

    if !resp.ok {
        anyhow::bail!(
            "setSuggestedPrompts error: {}",
            resp.error.unwrap_or_default()
        );
    }
    Ok(())
}

/// POST assistant.threads.setStatus
pub async fn set_thread_status(
    token: &str,
    channel_id: &str,
    thread_ts: &str,
    status: &str,
) -> Result<()> {
    let body = serde_json::json!({
        "channel_id": channel_id,
        "thread_ts": thread_ts,
        "status": status,
    });

    let resp: SlackResponse = reqwest::Client::new()
        .post(format!("{SLACK_API}/assistant.threads.setStatus"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .context("setStatus request failed")?
        .json()
        .await
        .context("setStatus response parse failed")?;

    if !resp.ok {
        anyhow::bail!("setStatus error: {}", resp.error.unwrap_or_default());
    }
    Ok(())
}

// ── response types ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SlackResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepliesResponse {
    ok: bool,
    error: Option<String>,
    messages: Option<Vec<ConversationMessage>>,
}
