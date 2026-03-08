//! Checks a watch subscription topic.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::telegram_push::send_telegram_push;

pub struct WatchCheckHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl WatchCheckHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

#[async_trait]
impl EventHandler for WatchCheckHandler {
    fn event_type(&self) -> EventType {
        EventType::WatchCheck
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let (subscription_id, topic) = match &event.payload {
            EventPayload::WatchCheck {
                subscription_id,
                topic,
            } => (subscription_id.clone(), topic.clone()),
            _ => return Err(TrustyError::Storage("wrong payload type".into())),
        };

        // Verify subscription is still active
        let sqlite = store.sqlite.clone();
        let sid = subscription_id.clone();
        let is_active =
            tokio::task::spawn_blocking(move || sqlite.get_watch_subscription_active(&sid))
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?;

        if !is_active {
            info!(
                "WatchCheck: subscription {} no longer active",
                subscription_id
            );
            return Ok(DispatchResult::Done);
        }

        let update = generate_watch_update(&self.openrouter_base, &self.openrouter_api_key, &topic)
            .await
            .unwrap_or_else(|_| format!("No new updates for '{}'.", topic));

        let msg = format!("Watch update for '{}': {}", topic, update);
        send_telegram_push(&store.sqlite, &msg).await?;

        let next_at = chrono::Utc::now().timestamp() + 24 * 3600;
        info!("WatchCheck sent for topic '{}'", topic);
        Ok(DispatchResult::Chain(vec![(
            EventType::WatchCheck,
            EventPayload::WatchCheck {
                subscription_id,
                topic,
            },
            next_at,
        )]))
    }
}

async fn generate_watch_update(base: &str, key: &str, topic: &str) -> Result<String, TrustyError> {
    let prompt = format!(
        "Give a brief 1-2 sentence update about the topic: '{}'. Be informative and concise.",
        topic
    );
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .json(&serde_json::json!({
            "model": "anthropic/claude-haiku-4-6",
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 150
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
        .unwrap_or("No update available.")
        .to_string())
}
