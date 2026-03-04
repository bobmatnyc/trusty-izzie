//! SQLite store for auth tokens, Gmail history cursors, and key-value config.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

use trusty_models::email::GmailHistoryCursor;
use trusty_models::{AgentTask, EventPayload, EventStatus, EventType, QueuedEvent};

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

            CREATE TABLE IF NOT EXISTS event_queue (
                id              TEXT PRIMARY KEY,
                event_type      TEXT NOT NULL,
                payload         TEXT NOT NULL DEFAULT '{}',
                status          TEXT NOT NULL DEFAULT 'pending'
                                CHECK(status IN ('pending','running','done','failed','cancelled')),
                priority        INTEGER NOT NULL DEFAULT 5,
                scheduled_at    INTEGER NOT NULL DEFAULT (unixepoch()),
                created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
                started_at      INTEGER,
                completed_at    INTEGER,
                attempts        INTEGER NOT NULL DEFAULT 0,
                max_retries     INTEGER NOT NULL DEFAULT 3,
                retry_after     INTEGER,
                error           TEXT,
                source          TEXT NOT NULL DEFAULT 'system',
                parent_event_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_eq_dispatch
                ON event_queue(status, scheduled_at ASC)
                WHERE status IN ('pending','failed');
            CREATE INDEX IF NOT EXISTS idx_eq_type ON event_queue(event_type, status);

            CREATE TABLE IF NOT EXISTS agent_tasks (
                id               TEXT PRIMARY KEY,
                agent_name       TEXT NOT NULL,
                task_description TEXT NOT NULL,
                status           TEXT NOT NULL DEFAULT 'pending',
                model            TEXT,
                output           TEXT,
                error            TEXT,
                created_at       INTEGER NOT NULL,
                started_at       INTEGER,
                completed_at     INTEGER,
                parent_event_id  TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_at_status ON agent_tasks(status, created_at DESC);

            CREATE TABLE IF NOT EXISTS accounts (
                id           TEXT PRIMARY KEY,
                email        TEXT NOT NULL UNIQUE,
                display_name TEXT,
                account_type TEXT NOT NULL DEFAULT 'secondary'
                             CHECK(account_type IN ('primary', 'secondary')),
                is_active    INTEGER NOT NULL DEFAULT 1,
                created_at   INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_accounts_active ON accounts (is_active, account_type);

            CREATE TABLE IF NOT EXISTS telegram_logs (
                id          TEXT PRIMARY KEY,
                direction   TEXT NOT NULL CHECK(direction IN ('inbound', 'outbound')),
                chat_id     INTEGER NOT NULL,
                user_id     INTEGER,
                username    TEXT,
                message     TEXT NOT NULL,
                tool_calls  TEXT,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_tg_logs_chat ON telegram_logs (chat_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_tg_logs_created ON telegram_logs (created_at DESC);

            CREATE TABLE IF NOT EXISTS user_prefs (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS vip_contacts (
                email TEXT PRIMARY KEY,
                name  TEXT,
                added_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS open_loops (
                id          TEXT PRIMARY KEY,
                description TEXT NOT NULL,
                context     TEXT,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                follow_up_at INTEGER NOT NULL,
                status      TEXT NOT NULL DEFAULT 'open'
                            CHECK(status IN ('open', 'dismissed', 'completed'))
            );

            CREATE TABLE IF NOT EXISTS watch_subscriptions (
                id          TEXT PRIMARY KEY,
                topic       TEXT NOT NULL,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                is_active   INTEGER NOT NULL DEFAULT 1
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
    /// Load the most recent `limit` messages for a session (chronological order).
    pub fn get_recent_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        // Fetch newest N rows first, then reverse for chronological order.
        let mut stmt = conn.prepare(
            "SELECT id, role, content, created_at FROM chat_messages \
             WHERE session_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        let mut messages: Vec<(String, String, String, i64)> =
            rows.filter_map(|r| r.ok()).collect();
        messages.reverse(); // oldest first
        Ok(messages)
    }

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

    // ── Event queue ───────────────────────────────────────────────────────────

    /// Insert a new event into the queue. Returns the new event's UUID string.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_event(
        &self,
        event_type: &EventType,
        payload: &EventPayload,
        scheduled_at: i64,
        priority: i64,
        max_retries: i64,
        source: &str,
        parent_event_id: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let payload_json = serde_json::to_string(payload)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO event_queue (id, event_type, payload, scheduled_at, priority, max_retries, source, parent_event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                event_type.as_str(),
                payload_json,
                scheduled_at,
                priority,
                max_retries,
                source,
                parent_event_id,
            ],
        )?;
        Ok(id)
    }

    /// Atomically claim the next dispatchable event, marking it `running`.
    ///
    /// Returns `None` if no claimable event exists.
    pub fn claim_next_event(&self) -> Result<Option<QueuedEvent>> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();

        let maybe_row = conn.query_row(
            "SELECT id, event_type, payload, priority, scheduled_at, created_at,
                    started_at, completed_at, attempts, max_retries, retry_after, error, source, parent_event_id
             FROM event_queue
             WHERE (status = 'pending' AND scheduled_at <= ?1)
                OR (status = 'failed' AND attempts < max_retries AND (retry_after IS NULL OR retry_after <= ?1))
             ORDER BY priority ASC, scheduled_at ASC
             LIMIT 1",
            rusqlite::params![now],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, Option<String>>(13)?,
                ))
            },
        );

        let (
            id,
            event_type_str,
            payload_json,
            priority,
            scheduled_at_ts,
            created_at_ts,
            started_at_ts,
            completed_at_ts,
            attempts,
            max_retries,
            retry_after_ts,
            error,
            source,
            parent_event_id_str,
        ) = match maybe_row {
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(row) => row,
        };

        // Atomically claim it; returns 0 if a concurrent worker already claimed it.
        let updated = conn.execute(
            "UPDATE event_queue SET status = 'running', started_at = ?1, attempts = attempts + 1
             WHERE id = ?2 AND status IN ('pending', 'failed')",
            rusqlite::params![now, id],
        )?;

        if updated == 0 {
            return Ok(None);
        }

        fn ts(ts: i64) -> DateTime<Utc> {
            DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now)
        }

        Ok(Some(QueuedEvent {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
            event_type: event_type_str
                .parse::<EventType>()
                .map_err(|e| anyhow::anyhow!(e))?,
            payload: serde_json::from_str(&payload_json)?,
            status: EventStatus::Running,
            priority,
            scheduled_at: ts(scheduled_at_ts),
            created_at: ts(created_at_ts),
            started_at: started_at_ts.map(ts),
            completed_at: completed_at_ts.map(ts),
            attempts: attempts + 1,
            max_retries,
            retry_after: retry_after_ts.map(ts),
            error,
            source,
            parent_event_id: parent_event_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        }))
    }

    /// Mark an event as successfully completed.
    pub fn complete_event(&self, id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE event_queue SET status = 'done', completed_at = ?1 WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
        Ok(())
    }

    /// Mark an event as failed. `retry_after` is a Unix timestamp; `None` means no retry.
    pub fn fail_event(&self, id: &str, error: &str, retry_after: Option<i64>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE event_queue SET status = 'failed', error = ?1, retry_after = ?2 WHERE id = ?3",
            rusqlite::params![error, retry_after, id],
        )?;
        Ok(())
    }

    /// Cancel a pending or failed event.
    pub fn cancel_event(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE event_queue SET status = 'cancelled'
             WHERE id = ?1 AND status IN ('pending', 'failed')",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    // ── Agent tasks ───────────────────────────────────────────────────────────

    /// Insert a new agent task in `pending` status.
    pub fn create_agent_task(
        &self,
        id: &str,
        agent_name: &str,
        task_description: &str,
        model: Option<&str>,
        parent_event_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_tasks (id, agent_name, task_description, status, model, created_at, parent_event_id)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
            rusqlite::params![id, agent_name, task_description, model, now, parent_event_id],
        )?;
        Ok(())
    }

    /// Update a task's status, output, and error. Also sets `started_at` when
    /// transitioning to `running` and `completed_at` for all other transitions.
    pub fn update_agent_task(
        &self,
        id: &str,
        status: &str,
        output: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agent_tasks SET status = ?1, output = ?2, error = ?3 WHERE id = ?4",
            rusqlite::params![status, output, error, id],
        )?;
        if status == "running" {
            conn.execute(
                "UPDATE agent_tasks SET started_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )?;
        } else {
            conn.execute(
                "UPDATE agent_tasks SET completed_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )?;
        }
        Ok(())
    }

    /// Retrieve a single agent task by ID.
    pub fn get_agent_task(&self, id: &str) -> Result<Option<AgentTask>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, agent_name, task_description, status, model, output, error,
                    created_at, started_at, completed_at, parent_event_id
             FROM agent_tasks WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(rusqlite::params![id], |row| {
                Ok(AgentTask {
                    id: row.get(0)?,
                    agent_name: row.get(1)?,
                    task_description: row.get(2)?,
                    status: row.get(3)?,
                    model: row.get(4)?,
                    output: row.get(5)?,
                    error: row.get(6)?,
                    created_at: row.get(7)?,
                    started_at: row.get(8)?,
                    completed_at: row.get(9)?,
                    parent_event_id: row.get(10)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// List agent tasks, optionally filtered by status, newest first.
    pub fn list_agent_tasks(&self, status: Option<&str>, limit: usize) -> Result<Vec<AgentTask>> {
        let conn = self.conn.lock().unwrap();

        fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTask> {
            Ok(AgentTask {
                id: row.get(0)?,
                agent_name: row.get(1)?,
                task_description: row.get(2)?,
                status: row.get(3)?,
                model: row.get(4)?,
                output: row.get(5)?,
                error: row.get(6)?,
                created_at: row.get(7)?,
                started_at: row.get(8)?,
                completed_at: row.get(9)?,
                parent_event_id: row.get(10)?,
            })
        }

        const COLS: &str = "id, agent_name, task_description, status, model, output, error,
                            created_at, started_at, completed_at, parent_event_id";

        let mut tasks = Vec::new();
        if let Some(s) = status {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM agent_tasks WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2"
            ))?;
            for row in stmt.query_map(rusqlite::params![s, limit as i64], map_row)? {
                tasks.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM agent_tasks ORDER BY created_at DESC LIMIT ?1"
            ))?;
            for row in stmt.query_map(rusqlite::params![limit as i64], map_row)? {
                tasks.push(row?);
            }
        }
        Ok(tasks)
    }

    // ── Accounts ──────────────────────────────────────────────────────────────

    /// Seed the primary account at startup (INSERT OR IGNORE — idempotent).
    pub fn seed_primary_account(&self, email: &str) -> Result<()> {
        let id = format!("primary-{}", email);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT OR IGNORE INTO accounts (id, email, display_name, account_type, is_active)
            VALUES (?1, ?2, ?2, 'primary', 1)
            "#,
            rusqlite::params![id, email],
        )?;
        Ok(())
    }

    /// List all accounts ordered by type ('primary' first), then created_at.
    pub fn list_accounts(&self) -> Result<Vec<trusty_models::Account>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, email, display_name, account_type, is_active, created_at
             FROM accounts
             ORDER BY CASE account_type WHEN 'primary' THEN 0 ELSE 1 END ASC, created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(trusty_models::Account {
                id: row.get(0)?,
                email: row.get(1)?,
                display_name: row.get(2)?,
                account_type: row.get(3)?,
                is_active: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
            })
        })?;
        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row?);
        }
        Ok(accounts)
    }

    /// Add or update an account (INSERT OR REPLACE).
    pub fn add_account(
        &self,
        email: &str,
        display_name: Option<&str>,
        account_type: &str,
    ) -> Result<()> {
        let id = format!("{}-{}", account_type, email);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO accounts (id, email, display_name, account_type, is_active)
            VALUES (?1, ?2, ?3, ?4, 1)
            ON CONFLICT(email) DO UPDATE SET
                display_name = excluded.display_name,
                account_type = excluded.account_type,
                is_active    = 1
            "#,
            rusqlite::params![id, email, display_name.unwrap_or(email), account_type],
        )?;
        Ok(())
    }

    /// Deactivate a secondary account (is_active = 0). Returns Err if primary or not found.
    pub fn deactivate_account(&self, email: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let account_type: Option<String> = conn
            .query_row(
                "SELECT account_type FROM accounts WHERE email = ?1",
                rusqlite::params![email],
                |row| row.get(0),
            )
            .optional()?;
        match account_type.as_deref() {
            None => return Err(anyhow::anyhow!("Account not found: {}", email)),
            Some("primary") => {
                return Err(anyhow::anyhow!("Cannot deactivate the primary account"))
            }
            _ => {}
        }
        conn.execute(
            "UPDATE accounts SET is_active = 0 WHERE email = ?1",
            rusqlite::params![email],
        )?;
        Ok(())
    }

    /// Get a single account by email.
    pub fn get_account(&self, email: &str) -> Result<Option<trusty_models::Account>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, email, display_name, account_type, is_active, created_at
             FROM accounts WHERE email = ?1",
        )?;
        let result = stmt
            .query_row(rusqlite::params![email], |row| {
                Ok(trusty_models::Account {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    display_name: row.get(2)?,
                    account_type: row.get(3)?,
                    is_active: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// Get OAuth token row by user_id (= email).
    pub fn get_oauth_token(&self, user_id: &str) -> Result<Option<trusty_models::OAuthToken>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT user_id, access_token, refresh_token, expires_at
             FROM oauth_tokens WHERE user_id = ?1",
        )?;
        let result = stmt
            .query_row(rusqlite::params![user_id], |row| {
                Ok(trusty_models::OAuthToken {
                    user_id: row.get(0)?,
                    access_token: row.get(1)?,
                    refresh_token: row.get(2)?,
                    expires_at: row.get(3)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    // ── Telegram logs ─────────────────────────────────────────────────────────

    /// Log a Telegram interaction (inbound message or outbound reply).
    pub fn log_telegram_interaction(
        &self,
        direction: &str, // "inbound" | "outbound"
        chat_id: i64,
        user_id: Option<i64>,
        username: Option<&str>,
        message: &str,
        tool_calls: Option<&str>,
    ) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO telegram_logs (id, direction, chat_id, user_id, username, message, tool_calls)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, direction, chat_id, user_id, username, message, tool_calls],
        )?;
        Ok(())
    }

    /// List events, optionally filtered by status, newest first.
    pub fn list_events(&self, status_filter: Option<&str>, limit: i64) -> Result<Vec<QueuedEvent>> {
        let conn = self.conn.lock().unwrap();

        fn ts(ts: i64) -> DateTime<Utc> {
            DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now)
        }

        fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedEvent> {
            let id_str: String = row.get(0)?;
            let event_type_str: String = row.get(1)?;
            let payload_json: String = row.get(2)?;
            let status_str: String = row.get(3)?;
            Ok(QueuedEvent {
                id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()),
                event_type: event_type_str
                    .parse::<EventType>()
                    .unwrap_or(EventType::EmailSync),
                payload: serde_json::from_str(&payload_json)
                    .unwrap_or(EventPayload::EmailSync { force: false }),
                status: status_str
                    .parse::<EventStatus>()
                    .unwrap_or(EventStatus::Pending),
                priority: row.get(4)?,
                scheduled_at: ts(row.get::<_, i64>(5)?),
                created_at: ts(row.get::<_, i64>(6)?),
                started_at: row.get::<_, Option<i64>>(7)?.map(ts),
                completed_at: row.get::<_, Option<i64>>(8)?.map(ts),
                attempts: row.get(9)?,
                max_retries: row.get(10)?,
                retry_after: row.get::<_, Option<i64>>(11)?.map(ts),
                error: row.get(12)?,
                source: row.get(13)?,
                parent_event_id: row
                    .get::<_, Option<String>>(14)?
                    .and_then(|s| Uuid::parse_str(&s).ok()),
            })
        }

        const COLS: &str = "id, event_type, payload, status, priority, scheduled_at, created_at,
                            started_at, completed_at, attempts, max_retries, retry_after, error, source, parent_event_id";

        let mut events = Vec::new();
        if let Some(status) = status_filter {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM event_queue WHERE status = ?1 ORDER BY scheduled_at DESC LIMIT ?2"
            ))?;
            for row in stmt.query_map(rusqlite::params![status, limit], map_row)? {
                events.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM event_queue ORDER BY scheduled_at DESC LIMIT ?1"
            ))?;
            for row in stmt.query_map(rusqlite::params![limit], map_row)? {
                events.push(row?);
            }
        }
        Ok(events)
    }

    // ── User preferences ──────────────────────────────────────────────────────

    pub fn get_pref(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM user_prefs WHERE key = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![key], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    pub fn set_pref(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO user_prefs (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn list_all_prefs(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, value FROM user_prefs ORDER BY key")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut prefs = Vec::new();
        for row in rows {
            prefs.push(row?);
        }
        Ok(prefs)
    }

    // ── VIP contacts ──────────────────────────────────────────────────────────

    pub fn upsert_vip_contact(&self, email: &str, name: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO vip_contacts (email, name) VALUES (?1, ?2)
             ON CONFLICT(email) DO UPDATE SET name = excluded.name",
            rusqlite::params![email, name],
        )?;
        Ok(())
    }

    pub fn list_vip_contacts(&self) -> Result<Vec<(String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT email, name FROM vip_contacts ORDER BY email")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut contacts = Vec::new();
        for row in rows {
            contacts.push(row?);
        }
        Ok(contacts)
    }

    pub fn remove_vip_contact(&self, email: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM vip_contacts WHERE email = ?1",
            rusqlite::params![email],
        )?;
        Ok(())
    }

    // ── Open loops ────────────────────────────────────────────────────────────

    pub fn create_open_loop(
        &self,
        id: &str,
        description: &str,
        context: Option<&str>,
        follow_up_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO open_loops (id, description, context, follow_up_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO NOTHING",
            rusqlite::params![id, description, context, follow_up_at],
        )?;
        Ok(())
    }

    pub fn list_open_loops(&self, status: Option<&str>) -> Result<Vec<trusty_models::OpenLoopRow>> {
        let conn = self.conn.lock().unwrap();
        let mut loops = Vec::new();
        if let Some(s) = status {
            let mut stmt = conn.prepare(
                "SELECT id, description, context, created_at, follow_up_at, status
                 FROM open_loops WHERE status = ?1 ORDER BY follow_up_at ASC",
            )?;
            for row in stmt.query_map(rusqlite::params![s], |row| {
                Ok(trusty_models::OpenLoopRow {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    context: row.get(2)?,
                    created_at: row.get(3)?,
                    follow_up_at: row.get(4)?,
                    status: row.get(5)?,
                })
            })? {
                loops.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, description, context, created_at, follow_up_at, status
                 FROM open_loops ORDER BY follow_up_at ASC",
            )?;
            for row in stmt.query_map([], |row| {
                Ok(trusty_models::OpenLoopRow {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    context: row.get(2)?,
                    created_at: row.get(3)?,
                    follow_up_at: row.get(4)?,
                    status: row.get(5)?,
                })
            })? {
                loops.push(row?);
            }
        }
        Ok(loops)
    }

    pub fn close_open_loop(&self, id: &str, status: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE open_loops SET status = ?1 WHERE id = ?2",
            rusqlite::params![status, id],
        )?;
        Ok(())
    }

    // ── Watch subscriptions ───────────────────────────────────────────────────

    pub fn add_watch_subscription(&self, id: &str, topic: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO watch_subscriptions (id, topic) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET topic = excluded.topic, is_active = 1",
            rusqlite::params![id, topic],
        )?;
        Ok(())
    }

    pub fn list_watch_subscriptions(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, topic FROM watch_subscriptions WHERE is_active = 1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut subs = Vec::new();
        for row in rows {
            subs.push(row?);
        }
        Ok(subs)
    }

    pub fn remove_watch_subscription(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE watch_subscriptions SET is_active = 0 WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    pub fn get_watch_subscription_active(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT is_active FROM watch_subscriptions WHERE id = ?1")?;
        let result: Option<i64> = stmt
            .query_row(rusqlite::params![id], |row| row.get(0))
            .optional()?;
        Ok(result.map(|v| v != 0).unwrap_or(false))
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

    // ── Event queue tests ─────────────────────────────────────────────────────

    #[test]
    fn test_enqueue_and_claim_event() {
        let (_dir, store) = open_temp_store();
        let now = Utc::now().timestamp();
        let id = store
            .enqueue_event(
                &EventType::EmailSync,
                &EventPayload::EmailSync { force: false },
                now - 1, // scheduled in the past so it's immediately claimable
                4,
                3,
                "test",
                None,
            )
            .unwrap();
        assert!(!id.is_empty());

        let event = store.claim_next_event().unwrap().unwrap();
        assert_eq!(event.event_type, EventType::EmailSync);
        assert_eq!(event.source, "test");
        assert_eq!(event.attempts, 1);
        assert_eq!(event.status, EventStatus::Running);

        // Queue is now empty (event claimed).
        assert!(store.claim_next_event().unwrap().is_none());
    }

    #[test]
    fn test_complete_event() {
        let (_dir, store) = open_temp_store();
        let now = Utc::now().timestamp();
        store
            .enqueue_event(
                &EventType::Reminder,
                &EventPayload::Reminder {
                    message: "hi".to_string(),
                    subtitle: None,
                    url: None,
                },
                now - 1,
                2,
                1,
                "test",
                None,
            )
            .unwrap();
        let event = store.claim_next_event().unwrap().unwrap();
        store.complete_event(&event.id.to_string()).unwrap();

        let done = store.list_events(Some("done"), 10).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].status, EventStatus::Done);
    }

    #[test]
    fn test_fail_event_and_retry() {
        let (_dir, store) = open_temp_store();
        let now = Utc::now().timestamp();
        store
            .enqueue_event(
                &EventType::MemoryDecay,
                &EventPayload::MemoryDecay { min_age_days: None },
                now - 1,
                8,
                2,
                "test",
                None,
            )
            .unwrap();
        let event = store.claim_next_event().unwrap().unwrap();
        let id_str = event.id.to_string();
        // Fail with retry_after in the past so it's immediately claimable again.
        store.fail_event(&id_str, "boom", Some(now - 1)).unwrap();

        // Should be claimable again since retry_after <= now and attempts (1) < max_retries (2).
        let retry = store.claim_next_event().unwrap().unwrap();
        assert_eq!(retry.attempts, 2);
        assert_eq!(retry.error.as_deref(), Some("boom"));
    }

    #[test]
    fn test_cancel_event() {
        let (_dir, store) = open_temp_store();
        let now = Utc::now().timestamp() + 3600; // scheduled in the future
        let id = store
            .enqueue_event(
                &EventType::CalendarRefresh,
                &EventPayload::CalendarRefresh { lookahead_days: 7 },
                now,
                6,
                3,
                "test",
                None,
            )
            .unwrap();
        store.cancel_event(&id).unwrap();

        let cancelled = store.list_events(Some("cancelled"), 10).unwrap();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].status, EventStatus::Cancelled);
    }

    #[test]
    fn test_list_events_no_filter() {
        let (_dir, store) = open_temp_store();
        let now = Utc::now().timestamp();
        for _ in 0..3 {
            store
                .enqueue_event(
                    &EventType::EmailSync,
                    &EventPayload::EmailSync { force: false },
                    now,
                    4,
                    3,
                    "test",
                    None,
                )
                .unwrap();
        }
        let all = store.list_events(None, 10).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_future_event_not_claimed() {
        let (_dir, store) = open_temp_store();
        let future = Utc::now().timestamp() + 9999;
        store
            .enqueue_event(
                &EventType::EmailSync,
                &EventPayload::EmailSync { force: false },
                future,
                4,
                3,
                "test",
                None,
            )
            .unwrap();
        // Should not be claimable yet.
        assert!(store.claim_next_event().unwrap().is_none());
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
