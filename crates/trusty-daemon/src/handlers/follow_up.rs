//! Sends a follow-up reminder for an open loop and marks it completed.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::telegram_push::send_telegram_push;

pub struct FollowUpHandler;

#[async_trait]
impl EventHandler for FollowUpHandler {
    fn event_type(&self) -> EventType {
        EventType::FollowUp
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let (open_loop_id, description) = match &event.payload {
            EventPayload::FollowUp {
                open_loop_id,
                description,
            } => (open_loop_id.clone(), description.clone()),
            _ => return Err(TrustyError::Storage("wrong payload type".into())),
        };

        let msg = format!("Following up: {} — did this get resolved?", description);
        send_telegram_push(&store.sqlite, &msg).await?;

        let sqlite = store.sqlite.clone();
        let lid = open_loop_id.clone();
        tokio::task::spawn_blocking(move || sqlite.close_open_loop(&lid, "completed"))
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

        info!("FollowUp sent and loop {} marked completed", open_loop_id);
        Ok(DispatchResult::Done)
    }
}
