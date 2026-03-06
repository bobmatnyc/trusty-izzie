//! Agent task model for trusty-izzie's long-term agent system.

use serde::{Deserialize, Serialize};

/// A persisted record of an agent execution task.
///
/// Lifecycle: `pending` → `running` → `done` | `error`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    /// Matches the filename stem in docs/agents/{agent_name}.md
    pub agent_name: String,
    pub task_description: String,
    /// Current lifecycle status: "pending", "running", "done", "error"
    pub status: String,
    /// The model override used for this task (if any); falls back to agent definition default.
    pub model: Option<String>,
    /// Final output text produced by the agent on success.
    pub output: Option<String>,
    /// Error message if status is "error".
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    /// The event_queue ID that triggered this task.
    pub parent_event_id: Option<String>,
}

/// A Google account registered for email sync.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub account_type: String, // "primary" | "secondary"
    pub is_active: bool,
    pub created_at: i64,
    pub identity: String, // "work" | "personal"
}

/// An OAuth2 token row from the `oauth_tokens` table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthToken {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

/// A row from the `open_loops` table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenLoopRow {
    pub id: String,
    pub description: String,
    pub context: Option<String>,
    pub created_at: i64,
    pub follow_up_at: i64,
    pub status: String,
}
