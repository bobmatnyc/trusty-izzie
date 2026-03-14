//! Handler for the `NeedsReauth` event.
//!
//! Fires when EmailSyncHandler detects that no Google access token is present.
//! Sends a PKCE OAuth link directly to the user's Telegram chat.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::warn;
use trusty_core::error::TrustyError;
use trusty_email::auth::{generate_pkce_pair, GoogleAuthClient};
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct NeedsReauthHandler;

#[async_trait]
impl EventHandler for NeedsReauthHandler {
    fn event_type(&self) -> EventType {
        EventType::NeedsReauth
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let reason = match &event.payload {
            EventPayload::NeedsReauth { reason } => reason.clone(),
            _ => "unknown".to_string(),
        };

        // Look up Telegram credentials from SQLite config.
        let sqlite = store.sqlite.clone();
        let (bot_token, chat_id_str) =
            tokio::task::spawn_blocking(move || -> Result<(String, String), TrustyError> {
                let token = sqlite
                    .get_config("telegram_bot_token")
                    .map_err(|e| TrustyError::Storage(e.to_string()))?
                    .unwrap_or_default();
                let chat = sqlite
                    .get_config("telegram_primary_chat_id")
                    .map_err(|e| TrustyError::Storage(e.to_string()))?
                    .unwrap_or_default();
                Ok((token, chat))
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))??;

        let chat_id: i64 = chat_id_str.parse().unwrap_or(0);

        if bot_token.is_empty() || chat_id == 0 {
            warn!("NeedsReauth: Telegram not configured (no bot_token or chat_id), skipping");
            return Ok(DispatchResult::Done);
        }

        // Generate a fresh PKCE pair and build the consent URL.
        let (verifier, challenge) = generate_pkce_pair();
        let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
        let client_secret = trusty_core::secrets::get("GOOGLE_CLIENT_SECRET").unwrap_or_default();
        let ngrok =
            std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
        let redirect_uri = format!("https://{ngrok}/api/auth/google/callback");
        let auth_url = GoogleAuthClient::new(client_id, client_secret, redirect_uri)
            .authorization_url_pkce(&challenge);

        // Persist verifier + pending chat_id so the callback can confirm via Telegram.
        let sqlite2 = store.sqlite.clone();
        let verifier_clone = verifier.clone();
        let chat_id_str_clone = chat_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), TrustyError> {
            sqlite2
                .set_config("oauth_pkce_verifier", &verifier_clone)
                .map_err(|e| TrustyError::Storage(e.to_string()))?;
            sqlite2
                .set_config("oauth_pending_chat_id", &chat_id_str_clone)
                .map_err(|e| TrustyError::Storage(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| TrustyError::Storage(e.to_string()))??;

        // Send the reauth link via Telegram Bot API.
        let text = format!(
            "🔐 Gmail reauthorization needed ({reason}).\n\nClick to reconnect:\n{auth_url}\n\nLink expires in ~10 minutes."
        );
        reqwest::Client::new()
            .post(format!(
                "https://api.telegram.org/bot{bot_token}/sendMessage"
            ))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": text,
            }))
            .send()
            .await
            .map_err(|e| TrustyError::Http(e.to_string()))?;

        Ok(DispatchResult::Done)
    }
}
