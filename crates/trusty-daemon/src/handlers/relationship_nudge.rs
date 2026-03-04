//! Reminds user to reconnect with a VIP contact.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::telegram_push::send_telegram_push;

pub struct RelationshipNudgeHandler;

#[async_trait]
impl EventHandler for RelationshipNudgeHandler {
    fn event_type(&self) -> EventType {
        EventType::RelationshipNudge
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let (email, name, last_contact_days) = match &event.payload {
            EventPayload::RelationshipNudge {
                email,
                name,
                last_contact_days,
            } => (email.clone(), name.clone(), *last_contact_days),
            _ => return Err(TrustyError::Storage("wrong payload type".into())),
        };

        let enabled = store
            .sqlite
            .get_pref("relationship_nudge_enabled")
            .unwrap_or(None)
            .unwrap_or_else(|| "true".to_string());
        if enabled != "true" {
            info!("RelationshipNudge disabled by user pref");
            return Ok(DispatchResult::Done);
        }

        let msg = format!(
            "You haven't been in touch with {} in {} days. Want to reach out?",
            name, last_contact_days
        );
        send_telegram_push(&store.sqlite, &msg).await?;

        let next_at = chrono::Utc::now().timestamp() + 7 * 24 * 3600;
        info!("RelationshipNudge sent for {}", name);
        Ok(DispatchResult::Chain(vec![(
            EventType::RelationshipNudge,
            EventPayload::RelationshipNudge {
                email,
                name,
                last_contact_days: last_contact_days + 7,
            },
            next_at,
        )]))
    }
}
