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
    Ok(())
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
