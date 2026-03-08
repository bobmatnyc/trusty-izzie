//! Weekly digest sent every Monday at 9am.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::scheduling::next_weekly_ts;
use crate::telegram_push::send_telegram_push;

pub struct WeeklyDigestHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl WeeklyDigestHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

#[async_trait]
impl EventHandler for WeeklyDigestHandler {
    fn event_type(&self) -> EventType {
        EventType::WeeklyDigest
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let enabled = store
            .sqlite
            .get_pref("weekly_digest_enabled")
            .unwrap_or(None)
            .unwrap_or_else(|| "true".to_string());
        if enabled != "true" {
            info!("WeeklyDigest disabled by user pref");
            return Ok(schedule_next_monday());
        }

        let digest = generate_weekly_digest(&self.openrouter_base, &self.openrouter_api_key)
            .await
            .unwrap_or_else(|_| "Happy Monday! Here's to a productive new week.".to_string());

        send_telegram_push(&store.sqlite, &digest).await?;
        info!("WeeklyDigest sent");

        Ok(schedule_next_monday())
    }
}

fn schedule_next_monday() -> DispatchResult {
    DispatchResult::Chain(vec![(
        EventType::WeeklyDigest,
        EventPayload::WeeklyDigest {},
        next_weekly_ts(chrono::Weekday::Mon, 9, 0),
    )])
}

async fn generate_weekly_digest(base: &str, key: &str) -> Result<String, TrustyError> {
    let prompt = "Generate a brief friendly weekly check-in message for a personal AI assistant user. Keep it 2-3 sentences. Acknowledge it's the start of a new week. Encourage them positively.";
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .json(&serde_json::json!({
            "model": "anthropic/claude-haiku-4.5",
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 200
        }))
        .send()
        .await
        .map_err(|e| TrustyError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| TrustyError::Serialization(e.to_string()))?;
    Ok(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("Happy Monday!")
        .to_string())
}
