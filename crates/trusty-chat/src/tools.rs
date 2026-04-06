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
    /// Update an existing Google Calendar event.
    UpdateCalendarEvent,
    /// Update the user's current location (used for travel time calculations).
    UpdateUserLocation,
    /// Mark a Google Task as complete.
    CompleteTask,
    /// Fetch upcoming Metro North train departures between two stations.
    GetTrainSchedule,
    /// Fetch current Metro North service alerts.
    GetTrainAlerts,
    /// Discover available skills by keyword — returns matching skill names and descriptions.
    SearchSkills,
    /// Search the web using the Brave Search API.
    WebSearch,
    FetchPage,
    /// Get weather forecast for a location (Open-Meteo).
    GetWeather,
    /// Get active NWS severe weather alerts for a location (US only).
    GetWeatherAlerts,
    /// Design and build a new skill using Opus (design) and Sonnet (implementation).
    CreateSkill,
    /// Send a new email via Gmail on behalf of the user.
    SendEmail,
    /// Reply to an existing Gmail thread on behalf of the user.
    ReplyEmail,
    /// Search emails across connected Gmail accounts.
    SearchEmails,
    /// Create a new task in a Google Task list.
    CreateTask,
    /// Search Slack channel/DM history by keyword.
    SearchSlack,
    /// Unified search across all personal data + web (memories, email, iMessage, Slack, calendar, tasks, web).
    SearchAll,
    /// List pending actions awaiting user approval.
    ListPendingActions,
    /// Approve a pending action by ID (executes it immediately).
    ApproveAction,
    /// Reject a pending action by ID.
    RejectAction,
    /// AI-optimized web search via Tavily (returns direct answer + sources).
    TavilySearch,
    /// Full web page extraction via Firecrawl (returns clean markdown).
    FirecrawlScrape,
    /// Browser automation via Skyvern — navigate websites, fill forms, extract data.
    SkyvernTask,
    /// Google search with structured results via SerpApi (knowledge graph, rich snippets, local results).
    SerpApiSearch,
    /// Report Izzie's own operational state: accounts, skills, knowledge base counts, integrations.
    GetIzzieStatus,
    /// List all inbox filter rules.
    ListInboxRules,
    /// Add a new inbox filter rule.
    AddInboxRule,
    /// Remove an inbox filter rule by ID.
    RemoveInboxRule,
}

impl ToolName {
    /// Return a human-friendly status string for display while the tool executes.
    pub fn friendly_status(&self) -> &'static str {
        match self {
            Self::SearchMemories => "\u{1f4ad} Searching my memory\u{2026}",
            Self::SearchEntities => "\u{1f50d} Looking up people and places\u{2026}",
            Self::GetEntityRelationships => "\u{1f517} Checking connections\u{2026}",
            Self::SaveMemory => "\u{1f4be} Saving to memory\u{2026}",
            Self::GetCalendarEvents | Self::ListEvents => {
                "\u{1f4c5} Checking your calendar\u{2026}"
            }
            Self::CreateCalendarEvent => "\u{1f4c5} Creating calendar event\u{2026}",
            Self::UpdateCalendarEvent => "\u{1f4c5} Updating calendar event\u{2026}",
            Self::SearchEmails => "\u{1f4e7} Searching email\u{2026}",
            Self::SendEmail | Self::ReplyEmail => "\u{2709}\u{fe0f} Composing email\u{2026}",
            Self::WebSearch | Self::TavilySearch | Self::SerpApiSearch => {
                "\u{1f50d} Searching the web\u{2026}"
            }
            Self::FetchPage | Self::FirecrawlScrape => "\u{1f310} Reading webpage\u{2026}",
            Self::SkyvernTask => "\u{1f916} Running browser task\u{2026}",
            Self::GetWeather | Self::GetWeatherAlerts => {
                "\u{1f324}\u{fe0f} Checking weather\u{2026}"
            }
            Self::GetTrainSchedule | Self::GetTrainAlerts => {
                "\u{1f682} Checking train schedule\u{2026}"
            }
            Self::SearchImessages => "\u{1f4ac} Searching iMessage\u{2026}",
            Self::SearchWhatsapp => "\u{1f4ac} Searching WhatsApp\u{2026}",
            Self::SearchSlack => "\u{1f4ac} Searching Slack\u{2026}",
            Self::SearchContacts => "\u{1f464} Looking up contacts\u{2026}",
            Self::SearchAll => "\u{1f50d} Searching everything\u{2026}",
            Self::ExecuteShellCommand => "\u{2699}\u{fe0f} Running command\u{2026}",
            Self::GetTaskLists | Self::GetTasks | Self::GetTasksBulk => {
                "\u{1f4cb} Checking tasks\u{2026}"
            }
            Self::CreateTask | Self::CompleteTask => "\u{2705} Updating tasks\u{2026}",
            Self::CheckServiceStatus | Self::GetIzzieStatus => "\u{1f4ca} Checking status\u{2026}",
            Self::SubmitGithubIssue => "\u{1f4dd} Filing GitHub issue\u{2026}",
            Self::ListAgents | Self::RunAgent | Self::GetAgentTask => {
                "\u{1f916} Managing agents\u{2026}"
            }
            Self::ScheduleEvent => "\u{23f0} Scheduling event\u{2026}",
            Self::SearchSkills | Self::CreateSkill => "\u{1f9e0} Working with skills\u{2026}",
            Self::ListAccounts | Self::AddAccount | Self::RemoveAccount => {
                "\u{1f4e8} Managing accounts\u{2026}"
            }
            _ => "\u{1f914} Thinking\u{2026}",
        }
    }
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
