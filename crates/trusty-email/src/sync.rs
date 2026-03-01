//! Incremental email sync coordinator.

use anyhow::Result;
use tracing::{info, warn};

use trusty_models::email::GmailHistoryCursor;

use crate::client::GmailClient;

/// Drives one incremental email sync cycle.
///
/// Delegates storage to the caller via a callback so the `trusty-email`
/// crate does not depend on `trusty-store` directly.
pub struct EmailSyncer {
    client: GmailClient,
}

impl EmailSyncer {
    /// Construct with a pre-authenticated Gmail client.
    pub fn new(client: GmailClient) -> Self {
        Self { client }
    }

    /// Run a single incremental sync pass, starting from `cursor`.
    ///
    /// The `on_message` callback is invoked for each decoded message so the
    /// daemon can pipe it to the extractor and then the store.
    ///
    /// Returns the updated cursor with the new `last_history_id`.
    pub async fn sync_incremental(
        &self,
        cursor: &GmailHistoryCursor,
        mut on_message: impl AsyncFnMut(trusty_models::email::EmailMessage) -> Result<()>,
    ) -> Result<GmailHistoryCursor> {
        let message_ids = self
            .client
            .list_history_since(&cursor.last_history_id)
            .await?;

        let mut processed = 0u32;

        for id in &message_ids {
            match self.client.get_message(id).await {
                Ok(msg) => {
                    on_message(msg).await?;
                    processed += 1;
                }
                Err(e) => {
                    warn!(message_id = %id, error = %e, "failed to fetch message, skipping");
                }
            }
        }

        let new_history_id = self.client.get_history_id().await?;
        info!(
            messages_processed = processed,
            history_id = %new_history_id,
            "sync pass complete"
        );

        Ok(GmailHistoryCursor {
            user_id: cursor.user_id.clone(),
            last_history_id: new_history_id,
            last_synced_at: chrono::Utc::now(),
            messages_processed: processed,
        })
    }
}
