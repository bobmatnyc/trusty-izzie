use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct EmailSyncHandler;

#[async_trait]
impl EventHandler for EmailSyncHandler {
    fn event_type(&self) -> EventType {
        EventType::EmailSync
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        info!("EmailSync tick (stub) — full implementation pending trusty-email");
        // TODO: call trusty_email::sync() when implemented

        // Re-schedule self at the configured interval (default: 5 minutes).
        let store_clone = store.sqlite.clone();
        let interval =
            tokio::task::spawn_blocking(move || store_clone.get_config("email_sync_interval_secs"))
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(300);

        let next_at = chrono::Utc::now().timestamp() + interval;
        Ok(DispatchResult::Chain(vec![(
            EventType::EmailSync,
            EventPayload::EmailSync { force: false },
            next_at,
        )]))
    }
}
