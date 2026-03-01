//! Email message and Gmail sync cursor types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A parsed email message ready for extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailMessage {
    /// Gmail message ID (from the API).
    pub id: String,
    /// Gmail thread ID.
    pub thread_id: String,
    /// Email `From` header.
    pub from: String,
    /// All `To` header addresses.
    pub to: Vec<String>,
    /// All `Cc` header addresses.
    pub cc: Vec<String>,
    /// All `Bcc` header addresses (may be empty for received mail).
    pub bcc: Vec<String>,
    /// Decoded `Subject` header.
    pub subject: String,
    /// Plain-text body (preferred over HTML).
    pub body_text: Option<String>,
    /// HTML body (fallback if plain-text is unavailable).
    pub body_html: Option<String>,
    /// MIME-decoded snippet from the Gmail API.
    pub snippet: Option<String>,
    /// Labels applied by Gmail (e.g. `SENT`, `INBOX`).
    pub labels: Vec<String>,
    /// Date/time from the `Date` header.
    pub date: DateTime<Utc>,
    /// Whether this message was sent by the authenticated user.
    pub is_sent: bool,
    /// Internal ingestion timestamp.
    pub ingested_at: DateTime<Utc>,
}

/// Persistent cursor for incremental Gmail history polling.
///
/// The Gmail History API returns changes since a `historyId`. We store
/// the last processed ID so the daemon can resume without re-scanning
/// the full mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailHistoryCursor {
    /// Google `sub` claim (user ID) this cursor belongs to.
    pub user_id: String,
    /// The highest Gmail `historyId` we have fully processed.
    pub last_history_id: String,
    /// Timestamp of the last successful sync.
    pub last_synced_at: DateTime<Utc>,
    /// Number of messages processed in the last sync run.
    pub messages_processed: u32,
}
