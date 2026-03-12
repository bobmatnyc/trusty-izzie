//! Tool definitions that the LLM can invoke during a chat turn.

use serde::{Deserialize, Serialize};

/// A tool the LLM may request the runtime to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name as passed to/from the LLM.
    pub name: ToolName,
    /// Human-readable description shown to the model.
    pub description: &'static str,
    /// JSON Schema for the tool's input parameters.
    pub parameters: serde_json::Value,
}

/// Enumeration of all available tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    /// Search memories by semantic query.
    SearchMemories,
    /// Look up entities in the knowledge graph.
    SearchEntities,
    /// Fetch all relationships for a named entity.
    GetEntityRelationships,
    /// Save a new memory to the store.
    SaveMemory,
    /// Schedule a future event (reminder, email sync, etc.).
    ScheduleEvent,
    /// Cancel a pending event by its ID.
    CancelEvent,
    /// List scheduled events, optionally filtered by status.
    ListEvents,
    /// Report the running status of all trusty-izzie launchd services.
    CheckServiceStatus,
    /// Return the compiled version of this binary.
    GetVersion,
    /// File a GitHub issue via the `gh` CLI.
    SubmitGithubIssue,
    /// List all available agent definitions.
    ListAgents,
    /// Enqueue an agent run task.
    RunAgent,
    /// Get the status and output of an agent task by ID.
    GetAgentTask,
    /// List all registered Google accounts.
    ListAccounts,
    /// Add a new Google account (returns OAuth URL; email resolved after consent).
    AddAccount,
    /// Deactivate a secondary Google account (stops syncing it).
    RemoveAccount,
    /// Queue a macOS Contacts (AddressBook) sync.
    SyncContacts,
    /// Queue an iMessage/SMS sync from the macOS Messages database.
    SyncMessages,
    /// Queue a WhatsApp sync (live DB or exported .txt file).
    SyncWhatsApp,
    /// Execute a bash shell command and return stdout/stderr.
    ExecuteShellCommand,
    /// Fetch upcoming calendar events from Google Calendar.
    GetCalendarEvents,
    /// Get user preferences for proactive features.
    GetPreferences,
    /// Set a user preference (key/value).
    SetPreference,
    /// Add a VIP contact (always notified of emails from them).
    AddVipContact,
    /// Remove a VIP contact.
    RemoveVipContact,
    /// List all VIP contacts.
    ListVipContacts,
    /// Add a topic watch subscription.
    AddWatchSubscription,
    /// Remove a watch subscription.
    RemoveWatchSubscription,
    /// List all watch subscriptions.
    ListWatchSubscriptions,
    /// List open loops (pending follow-ups).
    ListOpenLoops,
    /// Dismiss an open loop.
    DismissOpenLoop,
    /// Fetch the user's Google Task lists.
    GetTaskLists,
    /// Fetch tasks from a Google Task list (default: all incomplete tasks).
    GetTasks,
    /// Fetch ALL task lists and ALL tasks for one account in a single call.
    GetTasksBulk,
    /// Search iMessage history by contact or keyword (direct read from chat.db).
    SearchImessages,
    /// Search macOS Address Book contacts by name, email, or phone.
    SearchContacts,
    /// Search WhatsApp Desktop message history by contact or keyword.
    SearchWhatsapp,
    /// Create a new event on Google Calendar.
    CreateCalendarEvent,
    /// Mark a Google Task as complete.
    CompleteTask,
    /// Fetch upcoming Metro North train departures between two stations.
    GetTrainSchedule,
    /// Fetch current Metro North service alerts.
    GetTrainAlerts,
}

/// A parsed tool call request from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Which tool to invoke.
    pub name: ToolName,
    /// The arguments to pass (must match the tool's parameter schema).
    pub arguments: serde_json::Value,
}

/// The result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Which tool produced this result.
    pub name: ToolName,
    /// JSON-serialised result data.
    pub result: serde_json::Value,
    /// Whether the tool succeeded.
    pub success: bool,
    /// Optional error message if `success` is false.
    pub error: Option<String>,
}
