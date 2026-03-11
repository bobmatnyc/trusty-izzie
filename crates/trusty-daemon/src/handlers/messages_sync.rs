//! iMessage/SMS sync — reads macOS Messages database and extracts
//! relationship context from sent messages.

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

pub struct MessagesSyncHandler;

/// A single iMessage/SMS message record.
struct MessageRecord {
    rowid: i64,
    text: String,
    handle: String,
}

fn read_messages(
    db_path: &str,
    after_rowid: i64,
    limit: usize,
) -> anyhow::Result<Vec<MessageRecord>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        r#"SELECT m.ROWID, m.text, h.id
           FROM message m
           JOIN handle h ON m.handle_id = h.ROWID
           WHERE m.is_from_me = 1
             AND m.text IS NOT NULL
             AND m.text != ''
             AND m.ROWID > ?1
           ORDER BY m.ROWID ASC
           LIMIT ?2"#,
    )?;
    let rows = stmt.query_map(rusqlite::params![after_rowid, limit as i64], |row| {
        Ok(MessageRecord {
            rowid: row.get(0)?,
            text: row.get::<_, String>(1)?,
            handle: row.get::<_, String>(2)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

#[async_trait]
impl EventHandler for MessagesSyncHandler {
    fn event_type(&self) -> EventType {
        EventType::MessagesSync
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let force = match &event.payload {
            EventPayload::MessagesSync { force } => *force,
            _ => false,
        };

        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/masa".to_string());
        let db_path = format!("{home}/Library/Messages/chat.db");

        if !std::path::Path::new(&db_path).exists() {
            warn!("Messages DB not found at {db_path} — Full Disk Access may be required");
            return Ok(DispatchResult::Done);
        }

        let cursor_key = "messages_sync_last_rowid";
        let sqlite = store.sqlite.clone();
        let cursor_key_clone = cursor_key.to_string();
        let last_rowid: i64 = if force {
            0
        } else {
            tokio::task::spawn_blocking(move || sqlite.get_config(&cursor_key_clone))
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        };

        info!(after_rowid = last_rowid, "Reading iMessages/SMS");

        let messages = tokio::task::spawn_blocking({
            let db_path = db_path.clone();
            move || read_messages(&db_path, last_rowid, 200)
        })
        .await
        .map_err(|e| TrustyError::Storage(e.to_string()))?
        .map_err(|e| TrustyError::Storage(e.to_string()))?;

        if messages.is_empty() {
            info!("No new messages since rowid {last_rowid}");
            return Ok(DispatchResult::Done);
        }

        let mut max_rowid = last_rowid;
        let mut synced = 0usize;
        let user_id = store.lance.user_id.clone();
        let now = chrono::Utc::now();

        for msg in &messages {
            max_rowid = max_rowid.max(msg.rowid);
            let text = msg.text.trim().to_string();
            if text.is_empty() {
                continue;
            }

            let fp = format!("imsg:{}", msg.rowid);
            let entity_id = Uuid::new_v4().to_string();
            let sqlite = store.sqlite.clone();
            let fp_clone = fp.clone();
            let entity_id_clone = entity_id.clone();
            let handle_clone = msg.handle.clone();

            let is_new = tokio::task::spawn_blocking(move || {
                sqlite.upsert_fingerprint(&fp_clone, &entity_id_clone, "Message", &handle_clone)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

            if !is_new && !force {
                continue;
            }

            // Store the recipient handle as a Person entity if it looks like a
            // real identifier (phone or email — skip group IDs).
            let handle = &msg.handle;
            if !handle.contains('@') && !handle.starts_with('+') {
                // Likely a group chat ID; skip.
                continue;
            }

            let normalized = handle.to_lowercase();
            let entity_uuid = Uuid::parse_str(&entity_id).unwrap_or_else(|_| Uuid::new_v4());
            let context = format!(
                "iMessage to {handle}: {}",
                text.chars().take(200).collect::<String>()
            );

            let entity = Entity {
                id: entity_uuid,
                user_id: user_id.clone(),
                entity_type: EntityType::Person,
                value: handle.clone(),
                normalized: normalized.clone(),
                confidence: 0.7,
                source: "imessage".to_string(),
                source_id: Some(fp.clone()),
                context: Some(context),
                aliases: vec![],
                occurrence_count: 1,
                first_seen: now,
                last_seen: now,
                created_at: now,
            };

            if let Err(e) = store.lance.upsert_entity(&entity, vec![0.0f32; 384]).await {
                warn!(handle = %handle, error = %e, "Failed to upsert message entity");
            } else {
                synced += 1;
            }
        }

        // Persist cursor.
        let sqlite = store.sqlite.clone();
        let max_rowid_str = max_rowid.to_string();
        tokio::task::spawn_blocking(move || sqlite.set_config(cursor_key, &max_rowid_str))
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;

        info!(synced, new_max_rowid = max_rowid, "Messages sync complete");
        Ok(DispatchResult::Done)
    }
}
