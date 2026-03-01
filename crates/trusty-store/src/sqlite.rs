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
        todo!("implement cursor SELECT from sqlite")
    }

    /// Store an OAuth2 access/refresh token pair for `user_id`.
    pub fn upsert_oauth_token(
        &self,
        _user_id: &str,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _expires_at: Option<i64>,
        _scopes: Option<&str>,
    ) -> Result<()> {
        todo!("implement token upsert in sqlite")
    }

    /// Retrieve the stored OAuth2 access token for `user_id`.
    pub fn get_access_token(&self, _user_id: &str) -> Result<Option<String>> {
        todo!("implement token SELECT from sqlite")
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
}
