//! Background email sync loop — polls Gmail incrementally and runs entity extraction.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tracing::{error, info, warn};
use trusty_models::email::GmailHistoryCursor;

use trusty_email::{GmailClient, GoogleAuthClient};
use trusty_extractor::{is_noise_email, EntityExtractor, UserContext};
use trusty_store::Store;

use crate::er_persist::persist_extraction_result;

/// Run the incremental email sync loop indefinitely.
///
/// Syncs immediately if no sync has occurred or the last sync is older than
/// `interval_secs`. After each sync, sleeps for `interval_secs`.
pub async fn run_email_sync_loop(
    store: Arc<Store>,
    extractor: Arc<EntityExtractor>,
    user_context: UserContext,
    interval_secs: u64,
    min_occurrences: u32,
) {
    let interval = Duration::from_secs(interval_secs);

    loop {
        // Determine if we should sync now or wait.
        let should_sync_now = match store.sqlite.get_gmail_cursor(&user_context.user_id) {
            Ok(Some(ref cursor)) => {
                let elapsed = Utc::now()
                    .signed_duration_since(cursor.last_synced_at)
                    .num_seconds() as u64;
                elapsed >= interval_secs
            }
            Ok(None) => true, // Never synced — sync immediately.
            Err(e) => {
                warn!(error = %e, "failed to read gmail cursor; syncing anyway");
                true
            }
        };

        if !should_sync_now {
            tokio::time::sleep(interval).await;
            continue;
        }

        if let Err(e) = run_sync_cycle(&store, &extractor, &user_context, min_occurrences).await {
            error!(error = %e, "email sync cycle failed");
        }

        tokio::time::sleep(interval).await;
    }
}

/// Execute one complete sync cycle.
async fn run_sync_cycle(
    store: &Arc<Store>,
    extractor: &Arc<EntityExtractor>,
    user_context: &UserContext,
    min_occurrences: u32,
) -> Result<()> {
    // Retrieve the access token.
    let access_token = match get_fresh_token(store, user_context).await {
        Some(t) => t,
        None => {
            warn!("no Google access token available; skipping email sync");
            return Ok(());
        }
    };

    let client = GmailClient::new(access_token);

    // Retrieve or seed the cursor.
    let cursor = match store.sqlite.get_gmail_cursor(&user_context.user_id)? {
        Some(c) => c,
        None => {
            // First run — seed cursor with current historyId, process no messages.
            let history_id = client.get_history_id().await?;
            info!(history_id = %history_id, "seeding gmail cursor (first run)");
            let cursor = GmailHistoryCursor {
                user_id: user_context.user_id.clone(),
                last_history_id: history_id,
                last_synced_at: Utc::now(),
                messages_processed: 0,
            };
            store.sqlite.upsert_gmail_cursor(&cursor)?;
            return Ok(());
        }
    };

    // Fetch the list of new message IDs since the last cursor.
    let message_ids = client.list_history_since(&cursor.last_history_id).await?;
    let mut total_entities = 0usize;
    let mut total_staged = 0usize;
    let mut total_rels = 0usize;
    let mut processed = 0u32;

    for id in &message_ids {
        let msg = match client.get_message(id).await {
            Ok(m) => m,
            Err(e) => {
                warn!(message_id = %id, error = %e, "failed to fetch message, skipping");
                continue;
            }
        };

        if is_noise_email(&msg) {
            continue;
        }

        match extractor.extract_from_email(&msg, user_context).await {
            Ok(result) => match persist_extraction_result(&result, store, min_occurrences).await {
                Ok(stats) => {
                    total_entities += stats.entities_written;
                    total_staged += stats.entities_staged;
                    total_rels += stats.relationships_written;
                }
                Err(e) => {
                    warn!(message_id = %id, error = %e, "failed to persist extraction");
                }
            },
            Err(e) => {
                warn!(message_id = %id, error = %e, "extraction failed for message");
            }
        }
        processed += 1;
    }

    // Fetch updated history ID and write new cursor.
    let new_history_id = client.get_history_id().await?;
    let new_cursor = GmailHistoryCursor {
        user_id: user_context.user_id.clone(),
        last_history_id: new_history_id.clone(),
        last_synced_at: Utc::now(),
        messages_processed: processed,
    };
    store.sqlite.upsert_gmail_cursor(&new_cursor)?;

    info!(
        entities_written = total_entities,
        entities_staged = total_staged,
        relationships_written = total_rels,
        messages_processed = processed,
        history_id = %new_history_id,
        "email sync cycle complete"
    );

    Ok(())
}

/// Get a fresh access token, refreshing via OAuth if needed.
///
/// Returns `None` if no token is stored.
async fn get_fresh_token(store: &Arc<Store>, user_context: &UserContext) -> Option<String> {
    // Try the direct access token first.
    let access_token = store
        .sqlite
        .get_config("google_access_token")
        .ok()
        .flatten()
        .filter(|t| !t.is_empty());

    if access_token.is_some() {
        return access_token;
    }

    // Fall back to refreshing using the stored refresh token.
    let refresh_token = store
        .sqlite
        .get_config("google_refresh_token")
        .ok()
        .flatten()
        .filter(|t| !t.is_empty())?;

    let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();

    if client_id.is_empty() || client_secret.is_empty() {
        warn!("GOOGLE_CLIENT_ID or GOOGLE_CLIENT_SECRET not set; cannot refresh token");
        return None;
    }

    let auth = GoogleAuthClient::new(
        client_id,
        client_secret,
        "https://izzie.ngrok.dev/api/auth/google/callback".to_string(),
    );

    match auth.refresh_token(&refresh_token).await {
        Ok(token_set) => {
            // Persist the new access token.
            if let Err(e) = store
                .sqlite
                .set_config("google_access_token", &token_set.access_token)
            {
                warn!(user_id = %user_context.user_id, error = %e, "failed to store refreshed access token");
            }
            // If a new refresh token was returned, persist it too.
            if let Some(new_refresh) = token_set.refresh_token {
                let _ = store
                    .sqlite
                    .set_config("google_refresh_token", &new_refresh);
            }
            Some(token_set.access_token)
        }
        Err(e) => {
            warn!(user_id = %user_context.user_id, error = %e, "token refresh failed");
            None
        }
    }
}
