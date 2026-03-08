//! Sends a morning briefing to the user via Telegram at 8am local time.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::scheduling::next_time_of_day_ts;
use crate::telegram_push::send_telegram_push;

pub struct MorningBriefingHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl MorningBriefingHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

#[async_trait]
impl EventHandler for MorningBriefingHandler {
    fn event_type(&self) -> EventType {
        EventType::MorningBriefing
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let enabled = store
            .sqlite
            .get_pref("morning_briefing_enabled")
            .unwrap_or(None)
            .unwrap_or_else(|| "true".to_string());
        if enabled != "true" {
            info!("MorningBriefing disabled by user pref");
            return Ok(schedule_next_morning());
        }

        let briefing =
            generate_briefing(&self.openrouter_base, &self.openrouter_api_key, "morning")
                .await
                .unwrap_or_else(|_| "Good morning! Ready to help with your day.".to_string());

        send_telegram_push(&store.sqlite, &briefing).await?;
        info!("MorningBriefing sent");

        Ok(schedule_next_morning())
    }
}

fn schedule_next_morning() -> DispatchResult {
    DispatchResult::Chain(vec![(
        EventType::MorningBriefing,
        EventPayload::MorningBriefing {},
        next_time_of_day_ts(8, 0),
    )])
}

async fn generate_briefing(base: &str, key: &str, period: &str) -> Result<String, TrustyError> {
    let prompt = match period {
        "morning" => "Generate a brief, warm good morning message for a personal AI assistant. 2-3 sentences max. Be friendly and upbeat. No bullet points.",
        "evening" => "Generate a brief end-of-day message for a personal AI assistant. 2-3 sentences max. Warm, reflective tone.",
        _ => "Generate a brief greeting.",
    };

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
        .unwrap_or("Good morning!")
        .to_string())
}
