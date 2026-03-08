//! Sends an evening briefing to the user via Telegram at 6pm local time.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::scheduling::next_time_of_day_ts;
use crate::telegram_push::send_telegram_push;

pub struct EveningBriefingHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl EveningBriefingHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

#[async_trait]
impl EventHandler for EveningBriefingHandler {
    fn event_type(&self) -> EventType {
        EventType::EveningBriefing
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let enabled = store
            .sqlite
            .get_pref("evening_briefing_enabled")
            .unwrap_or(None)
            .unwrap_or_else(|| "true".to_string());
        if enabled != "true" {
            info!("EveningBriefing disabled by user pref");
            return Ok(schedule_next_evening());
        }

        let briefing = generate_evening_briefing(&self.openrouter_base, &self.openrouter_api_key)
            .await
            .unwrap_or_else(|_| "Good evening! Hope your day went well.".to_string());

        send_telegram_push(&store.sqlite, &briefing).await?;
        info!("EveningBriefing sent");

        Ok(schedule_next_evening())
    }
}

fn schedule_next_evening() -> DispatchResult {
    DispatchResult::Chain(vec![(
        EventType::EveningBriefing,
        EventPayload::EveningBriefing {},
        next_time_of_day_ts(18, 0),
    )])
}

async fn generate_evening_briefing(base: &str, key: &str) -> Result<String, TrustyError> {
    let prompt = "Generate a brief end-of-day message for a personal AI assistant. 2-3 sentences max. Warm, reflective tone. Wish them a good evening.";
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .json(&serde_json::json!({
            "model": "anthropic/claude-haiku-4-6",
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
        .unwrap_or("Good evening!")
        .to_string())
}
