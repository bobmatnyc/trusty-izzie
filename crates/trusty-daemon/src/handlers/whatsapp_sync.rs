//! WhatsApp sync — reads macOS WhatsApp ChatStorage.sqlite or parses an
//! exported chat .txt file, and extracts relationship context from sent messages.

use async_trait::async_trait;
use rusqlite::Connection;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_models::entity::{Entity, EntityType};
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;
use uuid::Uuid;

use super::{DispatchResult, EventHandler};

/// Primary WhatsApp DB path (macOS shared group container).
const WA_DB_PATH: &str =
    "Library/Group Containers/group.net.whatsapp.WhatsApp.shared/ChatStorage.sqlite";

pub struct WhatsAppSyncHandler;

struct WaMessage {
    pk: i64,
    text: String,
    /// Chat session partner name or JID.
    partner: String,
}

fn read_wa_messages(db_path: &str, after_pk: i64, limit: usize) -> anyhow::Result<Vec<WaMessage>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        r#"SELECT m.Z_PK, m.ZTEXT, COALESCE(s.ZPARTNERNAME, s.ZCONTACTJID, '')
           FROM ZWAMESSAGE m
           JOIN ZWACHATSESSION s ON m.ZCHATSESSION = s.Z_PK
           WHERE m.ZISFROMME = 1
             AND m.ZTEXT IS NOT NULL
             AND m.ZTEXT != ''
             AND m.Z_PK > ?1
           ORDER BY m.Z_PK ASC
           LIMIT ?2"#,
    )?;
    let rows = stmt.query_map(rusqlite::params![after_pk, limit as i64], |row| {
        Ok(WaMessage {
            pk: row.get(0)?,
            text: row.get::<_, String>(1)?,
            partner: row.get::<_, String>(2)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

/// Parse an exported WhatsApp chat .txt file.
/// Format: `[MM/DD/YYYY, HH:MM:SS AM] Sender: text`
fn parse_export(content: &str, my_name: &str) -> Vec<WaMessage> {
    let mut out = Vec::new();
    let mut pk: i64 = 0;
    let mut other_name = String::new();

    for line in content.lines() {
        // Find closing bracket of timestamp.
        let Some(bracket) = line.find(']') else {
            continue;
        };
        let rest = &line[bracket + 1..].trim_start_matches(':').trim_start();
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let sender = rest[..colon].trim();
        let text = rest[colon + 1..].trim();

        if text.is_empty() {
            continue;
        }

        // Track the other participant name for context.
        if sender != my_name && other_name.is_empty() {
            other_name = sender.to_string();
        }

        if sender == my_name {
            pk += 1;
            out.push(WaMessage {
                pk,
                text: text.to_string(),
                partner: other_name.clone(),
            });
        }
    }
    out
}

#[async_trait]
impl EventHandler for WhatsAppSyncHandler {
    fn event_type(&self) -> EventType {
        EventType::WhatsAppSync
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let (export_path, force) = match &event.payload {
            EventPayload::WhatsAppSync { export_path, force } => (export_path.clone(), *force),
            _ => (None, false),
        };

        let home = std::env::var("HOME").expect("HOME environment variable must be set");

        let messages: Vec<WaMessage> = if let Some(ref path) = export_path {
            // Parse exported .txt file.
            let content = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| TrustyError::Storage(format!("Cannot read export file: {e}")))?;
            let my_name = std::env::var("TRUSTY_USER_NAME").unwrap_or_else(|_| "Masa".to_string());
            parse_export(&content, &my_name)
        } else {
            // Try the live DB.
            let db_path = format!("{home}/{WA_DB_PATH}");
            if !std::path::Path::new(&db_path).exists() {
                warn!("WhatsApp DB not found at {db_path}");
                return Ok(DispatchResult::Done);
            }

            let cursor_key = "whatsapp_sync_last_pk";
            let sqlite = store.sqlite.clone();
            let ck = cursor_key.to_string();
            let last_pk: i64 = if force {
                0
            } else {
                tokio::task::spawn_blocking(move || sqlite.get_config(&ck))
                    .await
                    .map_err(|e| TrustyError::Storage(e.to_string()))?
                    .map_err(|e| TrustyError::Storage(e.to_string()))?
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0)
            };

            info!(after_pk = last_pk, "Reading WhatsApp messages");

            let msgs = tokio::task::spawn_blocking({
                let db = db_path.clone();
                move || read_wa_messages(&db, last_pk, 200)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

            // Persist cursor before processing.
            if let Some(max) = msgs.iter().map(|m| m.pk).max() {
                let sqlite = store.sqlite.clone();
                let val = max.to_string();
                tokio::task::spawn_blocking(move || sqlite.set_config(cursor_key, &val))
                    .await
                    .map_err(|e| TrustyError::Storage(e.to_string()))?
                    .map_err(|e| TrustyError::Storage(e.to_string()))?;
            }

            msgs
        };

        if messages.is_empty() {
            info!("No new WhatsApp messages to process");
            return Ok(DispatchResult::Done);
        }

        let user_id = store.lance.user_id.clone();
        let now = chrono::Utc::now();
        let mut synced = 0usize;

        for msg in &messages {
            let text = msg.text.trim().to_string();
            if text.is_empty() || msg.partner.is_empty() {
                continue;
            }

            let fp = format!("wa:{}", msg.pk);
            let entity_id = Uuid::new_v4().to_string();
            let sqlite = store.sqlite.clone();
            let fp_clone = fp.clone();
            let eid_clone = entity_id.clone();
            let partner_clone = msg.partner.clone();

            let is_new = tokio::task::spawn_blocking(move || {
                sqlite.upsert_fingerprint(&fp_clone, &eid_clone, "Person", &partner_clone)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

            if !is_new && !force {
                continue;
            }

            let normalized = msg.partner.to_lowercase();
            let entity_uuid = Uuid::parse_str(&entity_id).unwrap_or_else(|_| Uuid::new_v4());
            let context = format!(
                "WhatsApp to {}: {}",
                msg.partner,
                text.chars().take(200).collect::<String>()
            );

            let entity = Entity {
                id: entity_uuid,
                user_id: user_id.clone(),
                entity_type: EntityType::Person,
                value: msg.partner.clone(),
                normalized,
                confidence: 0.75,
                source: "whatsapp".to_string(),
                source_id: Some(fp.clone()),
                context: Some(context),
                aliases: vec![],
                occurrence_count: 1,
                first_seen: now,
                last_seen: now,
                created_at: now,
            };

            if let Err(e) = store.lance.upsert_entity(&entity, vec![0.0f32; 384]).await {
                warn!(partner = %msg.partner, error = %e, "Failed to upsert WhatsApp entity");
            } else {
                synced += 1;
            }
        }

        info!(synced, "WhatsApp sync complete");
        Ok(DispatchResult::Done)
    }
}
