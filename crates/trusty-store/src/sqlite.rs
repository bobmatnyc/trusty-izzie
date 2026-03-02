//! SQLite store for auth tokens, Gmail history cursors, and key-value config.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

use trusty_models::email::GmailHistoryCursor;

/// Handle to the SQLite relational database.
///
/// The inner `Mutex` serialises access because `rusqlite::Connection` is not `Send`.
/// For high-write workloads consider migrating to `tokio-rusqlite`.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (or create) the SQLite database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Run schema migrations to ensure all tables exist.
    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;

            CREATE TABLE IF NOT EXISTS oauth_tokens (
                user_id       TEXT PRIMARY KEY,
                access_token  TEXT NOT NULL,
                refresh_token TEXT,
                expires_at    INTEGER,  -- Unix timestamp
                scopes        TEXT,
                created_at    INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS gmail_cursors (
                user_id         TEXT PRIMARY KEY,
                last_history_id TEXT NOT NULL,
                last_synced_at  INTEGER NOT NULL,
                messages_processed INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS kv_config (
                key        TEXT PRIMARY KEY,
                value      TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS entity_fingerprints (
                fingerprint     TEXT PRIMARY KEY,
                entity_id       TEXT NOT NULL,
                entity_type     TEXT NOT NULL,
                normalized_name TEXT NOT NULL,
                first_seen      INTEGER NOT NULL DEFAULT (unixepoch()),
                last_seen       INTEGER NOT NULL DEFAULT (unixepoch()),
                seen_count      INTEGER NOT NULL DEFAULT 1,
                graduated       INTEGER NOT NULL DEFAULT 0,
                is_spam         INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_fp_normalized ON entity_fingerprints(entity_type, normalized_name);

            CREATE TABLE IF NOT EXISTS chat_sessions (
                id              TEXT PRIMARY KEY,
                title           TEXT,
                created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
                last_active_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                message_count   INTEGER NOT NULL DEFAULT 0,
                token_estimate  INTEGER NOT NULL DEFAULT 0,
                compressed_summary TEXT
            );

            CREATE TABLE IF NOT EXISTS chat_messages (
                id          TEXT PRIMARY KEY,
                session_id  TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
                role        TEXT NOT NULL CHECK(role IN ('user','assistant','tool')),
                content     TEXT NOT NULL,
                tool_calls  TEXT,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                tokens      INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_msgs_session ON chat_messages(session_id, created_at ASC);
            "#,
        )?;
        Ok(())
    }

    /// Persist or update a Gmail history cursor for `user_id`.
    pub fn upsert_gmail_cursor(&self, cursor: &GmailHistoryCursor) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO gmail_cursors (user_id, last_history_id, last_synced_at, messages_processed)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(user_id) DO UPDATE SET
                last_history_id    = excluded.last_history_id,
                last_synced_at     = excluded.last_synced_at,
                messages_processed = excluded.messages_processed
            "#,
            rusqlite::params![
                cursor.user_id,
                cursor.last_history_id,
                cursor.last_synced_at.timestamp(),
                cursor.messages_processed,
            ],
        )?;
        Ok(())
    }

    /// Retrieve the Gmail history cursor for `user_id`, if one exists.
    pub fn get_gmail_cursor(&self, user_id: &str) -> Result<Option<GmailHistoryCursor>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT user_id, last_history_id, last_synced_at, messages_processed FROM gmail_cursors WHERE user_id = ?1",
        )?;
        let result = stmt
            .query_row(rusqlite::params![user_id], |row| {
                let ts: i64 = row.get(2)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    ts,
                    row.get::<_, u32>(3)?,
                ))
            })
            .optional()?;

        match result {
            None => Ok(None),
            Some((uid, history_id, ts, msgs)) => {
                let last_synced_at = chrono::DateTime::from_timestamp(ts, 0)
                    .ok_or_else(|| anyhow::anyhow!("invalid timestamp {}", ts))?;
                Ok(Some(GmailHistoryCursor {
                    user_id: uid,
                    last_history_id: history_id,
                    last_synced_at,
                    messages_processed: msgs,
                }))
            }
        }
    }

    /// Store an OAuth2 access/refresh token pair for `user_id`.
    pub fn upsert_oauth_token(
        &self,
        user_id: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<i64>,
        scopes: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO oauth_tokens (user_id, access_token, refresh_token, expires_at, scopes)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(user_id) DO UPDATE SET
                access_token  = excluded.access_token,
                refresh_token = excluded.refresh_token,
                expires_at    = excluded.expires_at,
                scopes        = excluded.scopes
            "#,
            rusqlite::params![user_id, access_token, refresh_token, expires_at, scopes],
        )?;
        Ok(())
    }

    /// Retrieve the stored OAuth2 access token for `user_id`.
    pub fn get_access_token(&self, user_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT access_token FROM oauth_tokens WHERE user_id = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![user_id], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    /// Set a key-value config entry.
    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO kv_config (key, value) VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                                           updated_at = unixepoch()
            "#,
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Get a key-value config entry.
    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM kv_config WHERE key = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![key], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    /// Upsert an entity fingerprint. Returns `true` if this is a new fingerprint,
    /// `false` if it already existed (seen_count incremented).
    pub fn upsert_fingerprint(
        &self,
        fp: &str,
        entity_id: &str,
        entity_type: &str,
        normalized_name: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows_affected = conn.execute(
            r#"
            INSERT INTO entity_fingerprints (fingerprint, entity_id, entity_type, normalized_name)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(fingerprint) DO UPDATE SET
                seen_count = seen_count + 1,
                last_seen  = unixepoch()
            "#,
            rusqlite::params![fp, entity_id, entity_type, normalized_name],
        )?;
        // rows_affected is 1 for insert, 1 for update in SQLite upsert
        // Use changes() won't distinguish — check if it's brand new via returned rows
        // We detect "new" by checking if seen_count == 1 after the upsert
        let seen: u32 = conn.query_row(
            "SELECT seen_count FROM entity_fingerprints WHERE fingerprint = ?1",
            rusqlite::params![fp],
            |row| row.get(0),
        )?;
        let _ = rows_affected;
        Ok(seen == 1)
    }

    /// Return the current seen_count for a fingerprint (0 if not found).
    pub fn get_fingerprint_count(&self, fp: &str) -> Result<u32> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT seen_count FROM entity_fingerprints WHERE fingerprint = ?1")?;
        let result: Option<u32> = stmt
            .query_row(rusqlite::params![fp], |row| row.get(0))
            .optional()?;
        Ok(result.unwrap_or(0))
    }

    /// Mark a fingerprint as graduated (written to LanceDB + Kuzu).
    pub fn mark_fingerprint_graduated(&self, fp: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE entity_fingerprints SET graduated = 1 WHERE fingerprint = ?1",
            rusqlite::params![fp],
        )?;
        Ok(())
    }

    /// Mark a fingerprint as spam (skip forever).
    pub fn mark_fingerprint_spam(&self, fp: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE entity_fingerprints SET is_spam = 1 WHERE fingerprint = ?1",
            rusqlite::params![fp],
        )?;
        Ok(())
    }

    /// Create a new chat session.
    pub fn create_session(&self, id: &str, title: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO chat_sessions (id, title)
            VALUES (?1, ?2)
            ON CONFLICT(id) DO NOTHING
            "#,
            rusqlite::params![id, title],
        )?;
        Ok(())
    }

    /// Get a chat session by ID. Returns `(id, title, created_at)`.
    pub fn get_session(&self, id: &str) -> Result<Option<(String, Option<String>, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, title, created_at FROM chat_sessions WHERE id = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        Ok(result)
    }

    /// Add a message to a chat session.
    pub fn add_message(
        &self,
        id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        tokens: Option<i32>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO chat_messages (id, session_id, role, content, tokens)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            rusqlite::params![id, session_id, role, content, tokens],
        )?;
        // Bump session message_count and last_active_at
        conn.execute(
            r#"
            UPDATE chat_sessions
            SET message_count = message_count + 1,
                last_active_at = unixepoch()
            WHERE id = ?1
            "#,
            rusqlite::params![session_id],
        )?;
        Ok(())
    }

    /// List the N most recently active chat sessions.
    /// Returns `Vec<(id, title, last_active_at)>`.
    pub fn list_recent_sessions(&self, limit: usize) -> Result<Vec<(String, Option<String>, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, last_active_at FROM chat_sessions ORDER BY last_active_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    /// Get all messages for a session, ordered by created_at ASC.
    /// Returns `Vec<(id, role, content, created_at)>`.
    pub fn get_messages(&self, session_id: &str) -> Result<Vec<(String, String, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, role, content, created_at FROM chat_messages WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn open_temp_store() -> (TempDir, SqliteStore) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SqliteStore::open(&db_path).unwrap();
        (dir, store)
    }

    #[test]
    fn test_set_and_get_config() {
        let (_dir, store) = open_temp_store();
        store.set_config("foo", "bar").unwrap();
        let val = store.get_config("foo").unwrap();
        assert_eq!(val, Some("bar".to_string()));
        let missing = store.get_config("missing").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn test_upsert_and_get_gmail_cursor() {
        let (_dir, store) = open_temp_store();
        let cursor = GmailHistoryCursor {
            user_id: "user1".to_string(),
            last_history_id: "12345".to_string(),
            last_synced_at: Utc::now(),
            messages_processed: 42,
        };
        store.upsert_gmail_cursor(&cursor).unwrap();
        let fetched = store.get_gmail_cursor("user1").unwrap().unwrap();
        assert_eq!(fetched.user_id, "user1");
        assert_eq!(fetched.last_history_id, "12345");
        assert_eq!(fetched.messages_processed, 42);

        // Ensure non-existent returns None
        let missing = store.get_gmail_cursor("nobody").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_upsert_fingerprint_increments_count() {
        let (_dir, store) = open_temp_store();
        let is_new = store
            .upsert_fingerprint("fp1", "eid1", "Person", "alice")
            .unwrap();
        assert!(is_new);
        assert_eq!(store.get_fingerprint_count("fp1").unwrap(), 1);

        let is_new2 = store
            .upsert_fingerprint("fp1", "eid1", "Person", "alice")
            .unwrap();
        assert!(!is_new2);
        assert_eq!(store.get_fingerprint_count("fp1").unwrap(), 2);

        // Unknown fingerprint returns 0
        assert_eq!(store.get_fingerprint_count("nope").unwrap(), 0);
    }

    #[test]
    fn test_create_session_and_add_message() {
        let (_dir, store) = open_temp_store();
        store.create_session("sess1", Some("My Chat")).unwrap();

        let session = store.get_session("sess1").unwrap().unwrap();
        assert_eq!(session.0, "sess1");
        assert_eq!(session.1, Some("My Chat".to_string()));

        store
            .add_message("msg1", "sess1", "user", "hello", Some(5))
            .unwrap();
        store
            .add_message("msg2", "sess1", "assistant", "world", Some(10))
            .unwrap();

        let msgs = store.get_messages("sess1").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].1, "user");
        assert_eq!(msgs[0].2, "hello");
        assert_eq!(msgs[1].1, "assistant");
        assert_eq!(msgs[1].2, "world");

        // Missing session returns None
        assert!(store.get_session("nope").unwrap().is_none());
    }
}
