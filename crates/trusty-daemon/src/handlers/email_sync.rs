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

        // Check for a Google access token before attempting sync.
        let sqlite_check = store.sqlite.clone();
        let has_token =
            tokio::task::spawn_blocking(move || sqlite_check.get_config("google_access_token"))
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map(|t| !t.is_empty())
                .unwrap_or(false);

        if !has_token {
            info!("EmailSync: no google_access_token found, triggering NeedsReauth");
            return Ok(DispatchResult::Chain(vec![
                (
                    EventType::NeedsReauth,
                    EventPayload::NeedsReauth {
                        reason: "no_token".to_string(),
                    },
                    chrono::Utc::now().timestamp(),
                ),
                // Retry EmailSync in 10 minutes (after user completes auth).
                (
                    EventType::EmailSync,
                    EventPayload::EmailSync { force: false },
                    chrono::Utc::now().timestamp() + 600,
                ),
            ]));
        }

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
