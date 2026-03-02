use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct EntityExtractionHandler;

#[async_trait]
impl EventHandler for EntityExtractionHandler {
    fn event_type(&self) -> EventType {
        EventType::EntityExtraction
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        _store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        if let EventPayload::EntityExtraction { message_ids, .. } = &event.payload {
            info!(
                "EntityExtraction stub: {} message(s) to process",
                message_ids.len()
            );
            // TODO: call trusty_extractor::extract() when implemented
        }
        Ok(DispatchResult::Done)
    }
}
