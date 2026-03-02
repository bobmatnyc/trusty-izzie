//! Chat session persistence and lifecycle management.

use anyhow::Result;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

use trusty_models::chat::{ChatMessage, ChatSession, MessageRole};
use trusty_store::SqliteStore;

/// Manages the creation, loading, and persistence of chat sessions.
pub struct SessionManager {
    sqlite: Arc<SqliteStore>,
}

impl SessionManager {
    /// Construct with a shared SQLite store handle.
    pub fn new(sqlite: Arc<SqliteStore>) -> Self {
        Self { sqlite }
    }

    /// Create a blank new in-memory session (not yet persisted).
    pub fn new_session(user_id: &str) -> ChatSession {
        let now = chrono::Utc::now();
        ChatSession {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            title: None,
            messages: vec![],
            is_compressed: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Load an existing session by ID from SQLite.
    pub async fn load(&self, session_id: Uuid) -> Result<Option<ChatSession>> {
        let id_str = session_id.to_string();

        let session_row = self.sqlite.get_session(&id_str)?;
        let (_, title, created_at_unix) = match session_row {
            None => return Ok(None),
            Some(row) => row,
        };

        let created_at =
            chrono::DateTime::from_timestamp(created_at_unix, 0).unwrap_or_else(chrono::Utc::now);

        let message_rows = self.sqlite.get_messages(&id_str)?;
        let mut messages = Vec::with_capacity(message_rows.len());
        let mut updated_at = created_at;

        for (msg_id, role_str, content, msg_ts) in message_rows {
            let role = match role_str.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "tool" => MessageRole::Tool,
                _ => MessageRole::System,
            };
            let msg_created_at =
                chrono::DateTime::from_timestamp(msg_ts, 0).unwrap_or_else(chrono::Utc::now);

            if msg_created_at > updated_at {
                updated_at = msg_created_at;
            }

            let msg_uuid = msg_id.parse::<Uuid>().unwrap_or_else(|_| Uuid::new_v4());

            messages.push(ChatMessage {
                id: msg_uuid,
                session_id,
                role,
                content,
                tool_name: None,
                tool_result: None,
                token_count: None,
                created_at: msg_created_at,
            });
        }

        Ok(Some(ChatSession {
            id: session_id,
            // user_id is not stored in the SQLite schema; default to empty
            user_id: String::new(),
            title,
            messages,
            is_compressed: false,
            created_at,
            updated_at,
        }))
    }

    /// Persist a session and any messages not yet saved to SQLite.
    ///
    /// Uses `ON CONFLICT DO NOTHING` for the session row, then inserts
    /// each message individually (also idempotent via the primary key).
    pub async fn save(&self, session: &ChatSession) -> Result<()> {
        let session_id_str = session.id.to_string();
        self.sqlite
            .create_session(&session_id_str, session.title.as_deref())?;

        for msg in &session.messages {
            // Only persist roles that the SQLite CHECK constraint allows.
            let role_str = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
                // System messages are runtime-constructed; skip persistence.
                MessageRole::System => continue,
            };

            let tokens = msg.token_count.map(|t| t as i32);
            // Silently ignore duplicate-key errors — message already persisted.
            if let Err(e) = self.sqlite.add_message(
                &msg.id.to_string(),
                &session_id_str,
                role_str,
                &msg.content,
                tokens,
            ) {
                // SQLite UNIQUE constraint violation is expected on re-saves.
                let msg_str = e.to_string();
                if !msg_str.contains("UNIQUE") {
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// Compress a long session by summarising old messages.
    ///
    /// Not yet implemented — logs a warning and returns `Ok(())`.
    pub async fn compress(&self, _session: &mut ChatSession) -> Result<()> {
        warn!("session compression is not yet implemented; skipping");
        Ok(())
    }
}
