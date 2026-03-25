//! Shared utility for sending push messages to the user's Telegram.

use tracing::{error, info};
use trusty_core::error::TrustyError;
use trusty_store::SqliteStore;

/// Send a Telegram message to the primary user.
/// Reads bot_token from SQLite kv_config (telegram_bot_token) and
/// chat_id from kv_config (telegram_primary_chat_id).
/// Silently returns Ok(()) if no chat_id is configured yet.
pub async fn send_telegram_push(sqlite: &SqliteStore, text: &str) -> Result<(), TrustyError> {
    let bot_token = match sqlite
        .get_config("telegram_bot_token")
        .map_err(|e| TrustyError::Storage(e.to_string()))?
    {
        Some(t) => t,
        None => {
            error!("send_telegram_push: no telegram_bot_token configured");
            return Ok(());
        }
    };

    let chat_id = match sqlite
        .get_config("telegram_primary_chat_id")
        .map_err(|e| TrustyError::Storage(e.to_string()))?
    {
        Some(id) => id,
        None => {
            info!("send_telegram_push: no telegram_primary_chat_id yet — skipping push");
            return Ok(());
        }
    };

    let client = reqwest::Client::new();
    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);

    for chunk in chunk_text(text, 4000) {
        let resp = client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML"
            }))
            .send()
            .await
            .map_err(|e| TrustyError::Http(format!("Telegram push failed: {e}")))?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_else(|_| "unknown".into());
            error!("Telegram push error: {}", err);
        }
    }

    // Fan-out to Slack webhook if configured (fire-and-forget, never blocks Telegram).
    if let Err(e) = crate::slack_push::send_slack_push(text).await {
        error!("Slack push error: {e}");
    }

    Ok(())
}

/// Send a Telegram message and return the message_id from the API response.
pub async fn send_telegram_push_with_id(
    sqlite: &SqliteStore,
    text: &str,
) -> Result<Option<i64>, TrustyError> {
    let bot_token = match sqlite
        .get_config("telegram_bot_token")
        .map_err(|e| TrustyError::Storage(e.to_string()))?
    {
        Some(t) => t,
        None => {
            error!("send_telegram_push_with_id: no telegram_bot_token configured");
            return Ok(None);
        }
    };

    let chat_id = match sqlite
        .get_config("telegram_primary_chat_id")
        .map_err(|e| TrustyError::Storage(e.to_string()))?
    {
        Some(id) => id,
        None => {
            info!("send_telegram_push_with_id: no telegram_primary_chat_id yet — skipping push");
            return Ok(None);
        }
    };

    let client = reqwest::Client::new();
    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);

    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML"
        }))
        .send()
        .await
        .map_err(|e| TrustyError::Http(format!("Telegram push failed: {e}")))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| TrustyError::Http(format!("Telegram response parse failed: {e}")))?;

    if json["ok"].as_bool() == Some(true) {
        let message_id = json["result"]["message_id"].as_i64();
        Ok(message_id)
    } else {
        error!("Telegram push error: {}", json);
        Ok(None)
    }
}

/// Edit an existing Telegram message. Returns true if successful.
pub async fn edit_telegram_message(
    sqlite: &SqliteStore,
    message_id: i64,
    text: &str,
) -> Result<bool, TrustyError> {
    let bot_token = match sqlite
        .get_config("telegram_bot_token")
        .map_err(|e| TrustyError::Storage(e.to_string()))?
    {
        Some(t) => t,
        None => {
            error!("edit_telegram_message: no telegram_bot_token configured");
            return Ok(false);
        }
    };

    let chat_id = match sqlite
        .get_config("telegram_primary_chat_id")
        .map_err(|e| TrustyError::Storage(e.to_string()))?
    {
        Some(id) => id,
        None => {
            info!("edit_telegram_message: no telegram_primary_chat_id yet — skipping");
            return Ok(false);
        }
    };

    let client = reqwest::Client::new();
    let url = format!("https://api.telegram.org/bot{}/editMessageText", bot_token);

    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "parse_mode": "HTML"
        }))
        .send()
        .await
        .map_err(|e| TrustyError::Http(format!("Telegram edit failed: {e}")))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| TrustyError::Http(format!("Telegram edit response parse failed: {e}")))?;

    Ok(json["ok"].as_bool() == Some(true))
}

fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if current.len() + line.len() + 1 > max_len {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    chunks
}
