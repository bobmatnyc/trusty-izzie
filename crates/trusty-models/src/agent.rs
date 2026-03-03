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
