//! Chat session persistence and lifecycle management.

use anyhow::Result;
use uuid::Uuid;

use trusty_models::chat::ChatSession;

/// Manages the creation, loading, and persistence of chat sessions.
pub struct SessionManager;

impl SessionManager {
    /// Create a blank new session.
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

    /// Load an existing session by ID.
    pub async fn load(&self, _session_id: Uuid) -> Result<Option<ChatSession>> {
        todo!("load session from SQLite")
    }

    /// Persist a session (insert or update).
    pub async fn save(&self, _session: &ChatSession) -> Result<()> {
        todo!("persist session to SQLite")
    }

    /// Compress a long session by summarising old messages.
    ///
    /// Called automatically once the message count exceeds
    /// `session_compression_threshold`.
    pub async fn compress(&self, _session: &mut ChatSession) -> Result<()> {
        todo!("call LLM to summarise old messages and replace with a system summary")
    }
}
