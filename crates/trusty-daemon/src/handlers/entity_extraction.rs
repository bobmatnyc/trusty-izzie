use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_embeddings::embedder::global_embedder;
use trusty_extractor::{is_noise_email, EntityExtractor, ExtractorConfig, UserContext};
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct EntityExtractionHandler {
    openrouter_base: String,
    openrouter_api_key: String,
}

impl EntityExtractionHandler {
    pub fn new(openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            openrouter_base,
            openrouter_api_key,
        }
    }
}

#[async_trait]
impl EventHandler for EntityExtractionHandler {
    fn event_type(&self) -> EventType {
        EventType::EntityExtraction
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let message_ids = match &event.payload {
            EventPayload::EntityExtraction { message_ids, .. } => message_ids.clone(),
            _ => return Ok(DispatchResult::Done),
        };

        if message_ids.is_empty() {
            info!("EntityExtraction: no message_ids, nothing to do");
            return Ok(DispatchResult::Done);
        }

        info!(
            count = message_ids.len(),
            "EntityExtraction: processing messages"
        );

        // Resolve user context from SQLite.
        let sqlite = store.sqlite.clone();
        let (user_id, user_email, user_display_name) =
            tokio::task::spawn_blocking(move || -> anyhow::Result<(String, String, String)> {
                let uid = sqlite
                    .get_config("google_user_id")?
                    .unwrap_or_else(|| "unknown".to_string());
                let email = sqlite
                    .get_config("google_email")?
                    .unwrap_or_else(|| "unknown@example.com".to_string());
                let name = sqlite
                    .get_config("google_display_name")?
                    .unwrap_or_else(|| "Unknown User".to_string());
                Ok((uid, email, name))
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

        let user_context = UserContext {
            user_id,
            email: user_email.clone(),
            display_name: user_display_name,
        };

        // Build Gmail client from stored access token.
        let sqlite2 = store.sqlite.clone();
        let uid_for_token = user_email.clone();
        let access_token =
            tokio::task::spawn_blocking(move || sqlite2.get_access_token(&uid_for_token))
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?;

        let access_token = match access_token {
            Some(t) if !t.is_empty() => t,
            _ => {
                warn!("EntityExtraction: no access token, skipping");
                return Ok(DispatchResult::Done);
            }
        };

        let gmail = trusty_email::client::GmailClient::new(access_token);

        // Build extractor.
        let extractor = EntityExtractor::new(ExtractorConfig {
            base_url: self.openrouter_base.clone(),
            api_key: self.openrouter_api_key.clone(),
            model: std::env::var("EXTRACTION_MODEL")
                .unwrap_or_else(|_| "mistralai/mistral-small-3.1-24b-instruct".to_string()),
            max_tokens: 1024,
            confidence_threshold: 0.85,
            max_relationships: 10,
        });

        let mut total_entities_stored = 0usize;
        let mut total_relationships_stored = 0usize;

        for message_id in &message_ids {
            // Fetch email from Gmail API.
            let email = match gmail.get_message(message_id).await {
                Ok(e) => e,
                Err(err) => {
                    warn!(message_id = %message_id, error = %err, "EntityExtraction: failed to fetch message, skipping");
                    continue;
                }
            };

            // Skip noise emails before calling the LLM.
            if is_noise_email(&email) {
                info!(message_id = %message_id, "EntityExtraction: skipping noise email");
                continue;
            }

            // Run LLM extraction.
            let result = match extractor.extract_from_email(&email, &user_context).await {
                Ok(r) => r,
                Err(err) => {
                    warn!(message_id = %message_id, error = %err, "EntityExtraction: extraction failed, skipping");
                    continue;
                }
            };

            if result.skipped_noise {
                info!(message_id = %message_id, "EntityExtraction: LLM skipped as noise");
                continue;
            }

            info!(
                message_id = %message_id,
                entities = result.entities.len(),
                relationships = result.relationships.len(),
                tokens = result.tokens_used,
                "EntityExtraction: extracted"
            );

            // Persist entities — deduplicate via fingerprint table.
            for entity in &result.entities {
                let fp = format!("entity:{:?}:{}", entity.entity_type, entity.normalized);
                let entity_id_str = entity.id.to_string();
                let normalized_clone = entity.normalized.clone();
                let type_str = format!("{:?}", entity.entity_type);

                let sqlite3 = store.sqlite.clone();
                let fp_clone = fp.clone();
                let eid_clone = entity_id_str.clone();
                let is_new = tokio::task::spawn_blocking(move || {
                    sqlite3.upsert_fingerprint(&fp_clone, &eid_clone, &type_str, &normalized_clone)
                })
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?;

                if !is_new {
                    // Entity already exists; skip re-embedding and re-inserting.
                    continue;
                }

                // Embed and store new entity.
                let embed_text = format!("{} {}", entity.value, entity.normalized);
                let embedding = tokio::task::spawn_blocking(move || {
                    if let Some(embedder) = global_embedder() {
                        embedder
                            .embed(&embed_text)
                            .unwrap_or_else(|_| vec![0.0f32; 384])
                    } else {
                        vec![0.0f32; 384]
                    }
                })
                .await
                .unwrap_or_else(|_| vec![0.0f32; 384]);

                if let Err(e) = store.lance.upsert_entity(entity, embedding).await {
                    warn!(entity = %entity.value, error = %e, "EntityExtraction: LanceDB upsert failed");
                }

                if let Some(graph) = store.graph.get().await {
                    let entity_clone = entity.clone();
                    if let Err(e) =
                        tokio::task::spawn_blocking(move || graph.upsert_entity(&entity_clone))
                            .await
                    {
                        warn!(entity = %entity.value, error = %e, "EntityExtraction: Kuzu upsert failed");
                    }
                } else {
                    warn!(entity = %entity.value, "EntityExtraction: KuzuDB not available");
                }

                total_entities_stored += 1;
            }

            // Persist relationships to Kuzu only (no vector embedding needed).
            for rel in &result.relationships {
                if let Some(graph) = store.graph.get().await {
                    let rel_clone = rel.clone();
                    if let Err(e) =
                        tokio::task::spawn_blocking(move || graph.upsert_relationship(&rel_clone))
                            .await
                    {
                        warn!(
                            from = %rel.from_entity_value,
                            to = %rel.to_entity_value,
                            error = %e,
                            "EntityExtraction: Kuzu relationship upsert failed"
                        );
                    } else {
                        total_relationships_stored += 1;
                    }
                }
            }
        }

        info!(
            entities_stored = total_entities_stored,
            relationships_stored = total_relationships_stored,
            "EntityExtraction: complete"
        );

        Ok(DispatchResult::Done)
    }
}
