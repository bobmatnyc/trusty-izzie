//! Shared utility for sending push messages via Slack incoming webhook.
//!
//! Reads SLACK_WEBHOOK_URL from environment (set at startup via .env).
//! Silently skips if the env var is absent — Slack is optional.

use tracing::{error, info};
use trusty_core::error::TrustyError;

/// Post a plain-text message to the configured Slack incoming webhook.
/// Markdown in `text` is rendered as Slack mrkdwn automatically.
pub async fn send_slack_push(text: &str) -> Result<(), TrustyError> {
    let webhook_url = match std::env::var("SLACK_WEBHOOK_URL")
        .ok()
        .filter(|s| !s.is_empty())
    {
        Some(u) => u,
        None => {
            info!("send_slack_push: SLACK_WEBHOOK_URL not set — skipping");
            return Ok(());
        }
    };

    let client = reqwest::Client::new();

    // Slack incoming webhooks accept blocks or plain text.
    // Use blocks so long briefings render with section dividers.
    let sections = split_sections(text, 3000);
    let blocks: Vec<serde_json::Value> = sections
        .iter()
        .map(|s| {
            serde_json::json!({
                "type": "section",
                "text": { "type": "mrkdwn", "text": s }
            })
        })
        .collect();

    let body = serde_json::json!({ "blocks": blocks });

    let resp = client
        .post(&webhook_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| TrustyError::Http(format!("Slack webhook POST failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_else(|_| "unknown".into());
        error!("Slack webhook error {}: {}", status, err);
    }

    Ok(())
}

/// Split text into chunks that fit within Slack's 3000-char block limit,
/// breaking on newlines rather than mid-word.
fn split_sections(text: &str, max_len: usize) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        // +1 for the newline we'll add
        if current.len() + line.len() + 1 > max_len && !current.is_empty() {
            sections.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        sections.push(current.trim().to_string());
    }
    if sections.is_empty() {
        sections.push(text.to_string());
    }
    sections
}
