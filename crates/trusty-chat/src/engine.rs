//! The core chat completion engine.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trusty_email::auth::GoogleAuthClient;
use trusty_models::chat::{ChatMessage, ChatSession, MessageRole, StructuredResponse};
use trusty_models::{EventPayload, EventType};
use trusty_store::SqliteStore;

use crate::context::ContextAssembler;
use crate::tools::ToolName;

const PRIMARY_EMAIL: &str = "bob@matsuoka.com";

/// Drives the conversation loop: context assembly → LLM call → tool dispatch → save.
pub struct ChatEngine {
    http: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
    /// Maximum number of tool call iterations per chat turn.
    max_tool_iterations: u32,
    context_assembler: ContextAssembler,
    /// Optional SQLite store for event queue tool dispatch.
    sqlite: Option<Arc<SqliteStore>>,
    /// Directory containing agent definition Markdown files.
    agents_dir: PathBuf,
}

// ── OpenRouter request/response types ────────────────────────────────────────

#[derive(Serialize)]
struct OrchatRequest<'a> {
    model: &'a str,
    messages: Vec<OrchatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    max_tokens: u32,
    temperature: f32,
    response_format: ResponseFormat,
}

#[derive(Serialize, Deserialize, Clone)]
struct OrchatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ResponseFormat {
    r#type: &'static str,
}

#[derive(Deserialize)]
struct OrchatResponse {
    choices: Vec<OrchatChoice>,
    usage: Option<OrchatUsage>,
}

#[derive(Deserialize)]
struct OrchatChoice {
    message: OrchatAssistantMsg,
}

#[derive(Deserialize)]
struct OrchatAssistantMsg {
    content: String,
}

#[derive(Deserialize)]
struct OrchatUsage {
    total_tokens: u32,
}

// ── ChatEngine impl ───────────────────────────────────────────────────────────

impl ChatEngine {
    /// Construct the chat engine with a default (empty) context assembler.
    pub fn new(api_base: String, api_key: String, model: String, max_tool_iterations: u32) -> Self {
        Self::new_with_context(
            api_base,
            api_key,
            model,
            max_tool_iterations,
            ContextAssembler::new(5, 10),
        )
    }

    /// Construct the chat engine with a fully configured context assembler.
    pub fn new_with_context(
        api_base: String,
        api_key: String,
        model: String,
        max_tool_iterations: u32,
        context_assembler: ContextAssembler,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");
        Self {
            http,
            api_base,
            api_key,
            model,
            max_tool_iterations,
            context_assembler,
            sqlite: None,
            agents_dir: PathBuf::from("docs/agents"),
        }
    }

    /// Attach a `SqliteStore` for event-queue tool dispatch.
    pub fn with_sqlite(mut self, sqlite: Arc<SqliteStore>) -> Self {
        self.sqlite = Some(sqlite);
        self
    }

    /// Set the agents directory for agent-related tools.
    pub fn with_agents_dir(mut self, agents_dir: PathBuf) -> Self {
        self.agents_dir = agents_dir;
        self
    }

    /// Execute a chat tool call and return the result as a string.
    ///
    /// Returns an error string rather than propagating `Err` so the model can
    /// receive feedback about what went wrong.
    pub fn execute_tool(&self, name: &ToolName, input: &serde_json::Value) -> Result<String> {
        match name {
            ToolName::ScheduleEvent => self.tool_schedule_event(input),
            ToolName::CancelEvent => self.tool_cancel_event(input),
            ToolName::ListEvents => self.tool_list_events(input),
            ToolName::CheckServiceStatus => self.tool_check_service_status(),
            ToolName::GetVersion => self.tool_get_version(),
            ToolName::SubmitGithubIssue => self.tool_submit_github_issue(input),
            ToolName::ListAgents => self.tool_list_agents(),
            ToolName::RunAgent => self.tool_run_agent(input),
            ToolName::GetAgentTask => self.tool_get_agent_task(input),
            ToolName::ListAccounts => self.tool_list_accounts(),
            ToolName::AddAccount => self.tool_add_account(),
            ToolName::RemoveAccount => self.tool_remove_account(input),
            ToolName::SyncContacts => self.tool_sync_contacts(),
            ToolName::SyncMessages => self.tool_sync_messages(),
            ToolName::SyncWhatsApp => self.tool_sync_whatsapp(input),
            ToolName::ExecuteShellCommand => self.tool_execute_shell_command(input),
            ToolName::GetCalendarEvents => self.tool_get_calendar_events(input),
            ToolName::GetPreferences => self.tool_get_preferences(),
            ToolName::SetPreference => self.tool_set_preference(input),
            ToolName::AddVipContact => self.tool_add_vip_contact(input),
            ToolName::RemoveVipContact => self.tool_remove_vip_contact(input),
            ToolName::ListVipContacts => self.tool_list_vip_contacts(),
            ToolName::AddWatchSubscription => self.tool_add_watch_subscription(input),
            ToolName::RemoveWatchSubscription => self.tool_remove_watch_subscription(input),
            ToolName::ListWatchSubscriptions => self.tool_list_watch_subscriptions(),
            ToolName::ListOpenLoops => self.tool_list_open_loops(),
            ToolName::DismissOpenLoop => self.tool_dismiss_open_loop(input),
            ToolName::GetTaskLists => self.tool_get_task_lists(),
            ToolName::GetTasks => self.tool_get_tasks(input),
            ToolName::SearchImessages => self.tool_search_imessages(input),
            ToolName::SearchContacts => self.tool_search_contacts(input),
            ToolName::SearchWhatsapp => self.tool_search_whatsapp(input),
            _ => Ok("Tool not yet implemented.".to_string()),
        }
    }

    fn sqlite_ref(&self) -> Result<&SqliteStore> {
        self.sqlite
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Event queue unavailable: no SQLite store attached"))
    }

    /// Return a valid (non-expired) access token for `user_id`, refreshing if needed.
    ///
    /// Refreshes if the token expires within the next 5 minutes. Returns `Err` if
    /// no token is stored or if the refresh fails.
    fn get_valid_token(&self, user_id: &str) -> Result<String> {
        let sqlite = self.sqlite_ref()?;
        let token = sqlite
            .get_oauth_token(user_id)?
            .ok_or_else(|| anyhow::anyhow!("No OAuth token stored for {}", user_id))?;

        // If token has > 5 minutes remaining, return as-is.
        let needs_refresh = token
            .expires_at
            .map(|exp| exp - chrono::Utc::now().timestamp() < 300)
            .unwrap_or(false);

        if !needs_refresh {
            return Ok(token.access_token);
        }

        let refresh_token = token
            .refresh_token
            .ok_or_else(|| anyhow::anyhow!("No refresh token for {}; re-auth required", user_id))?;

        let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
        let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();
        let ngrok =
            std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
        let redirect_uri = format!("https://{}/api/auth/google/callback", ngrok);

        let auth = GoogleAuthClient::new(client_id, client_secret, redirect_uri);

        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| anyhow::anyhow!("No async runtime available for token refresh"))?;

        let new_token = tokio::task::block_in_place(|| {
            handle.block_on(async { auth.refresh_token(&refresh_token).await })
        })?;

        let new_expires_at = Some(chrono::Utc::now().timestamp() + new_token.expires_in as i64);

        sqlite.refresh_oauth_token(
            user_id,
            &new_token.access_token,
            new_token.refresh_token.as_deref(),
            new_expires_at,
        )?;

        tracing::info!(user_id, "OAuth token refreshed");
        Ok(new_token.access_token)
    }

    /// Build a system-prompt section describing the currently connected Google accounts.
    ///
    /// Returns an empty string if no SQLite store is attached or no accounts exist.
    fn load_accounts_context(&self) -> String {
        let sqlite = match self.sqlite.as_deref() {
            Some(s) => s,
            None => return String::new(),
        };

        let accounts = match sqlite.list_accounts() {
            Ok(a) => a,
            Err(_) => return String::new(),
        };

        let active: Vec<_> = accounts.iter().filter(|a| a.is_active).collect();
        if active.is_empty() {
            return String::new();
        }

        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());

        let mut lines = vec!["## Connected Google Accounts".to_string()];
        for acc in &active {
            // Primary account has full scope (Calendar, Tasks, Drive, Gmail).
            // Secondary accounts were added with Gmail + userinfo scope only.
            let capabilities = if acc.email == primary_email {
                "Gmail · Calendar · Tasks · Drive"
            } else {
                "Gmail only"
            };
            let role = if acc.account_type == "primary" {
                "primary"
            } else {
                "secondary"
            };
            lines.push(format!("- **{}** ({}) — {}", acc.email, role, capabilities));
        }
        lines.push(String::new());
        lines.push(format!(
            "Calendar and Tasks always use **{}** (the only account with those scopes).",
            primary_email
        ));
        lines.push("Email relationship context covers all connected accounts.".to_string());

        lines.join("\n")
    }

    fn tool_schedule_event(&self, input: &serde_json::Value) -> Result<String> {
        let event_type_str = input["event_type"].as_str().unwrap_or("");
        let scheduled_at_str = input["scheduled_at"].as_str().unwrap_or("");

        let scheduled_at = chrono::DateTime::parse_from_rfc3339(scheduled_at_str)
            .map_err(|_| anyhow::anyhow!("Invalid scheduled_at format, use ISO 8601"))?;
        let now = chrono::Utc::now();
        if scheduled_at.with_timezone(&chrono::Utc) <= now {
            return Ok("Error: scheduled_at must be in the future".to_string());
        }

        let event_type = event_type_str
            .parse::<EventType>()
            .map_err(|e| anyhow::anyhow!(e))?;

        let payload = match &event_type {
            EventType::Reminder => EventPayload::Reminder {
                message: input["message"].as_str().unwrap_or("Reminder").to_string(),
                subtitle: input["subtitle"].as_str().map(|s| s.to_string()),
                url: None,
            },
            EventType::EmailSync => EventPayload::EmailSync { force: false },
            EventType::MemoryDecay => EventPayload::MemoryDecay { min_age_days: None },
            EventType::CalendarRefresh => EventPayload::CalendarRefresh { lookahead_days: 7 },
            EventType::EntityExtraction => EventPayload::EntityExtraction {
                message_ids: vec![],
                source_event_id: None,
            },
            EventType::NeedsReauth => {
                return Ok(
                    "NeedsReauth is a system event and cannot be scheduled from chat.".to_string(),
                )
            }
            EventType::AgentRun => EventPayload::AgentRun {
                agent_name: input["agent_name"]
                    .as_str()
                    .unwrap_or("summarizer")
                    .to_string(),
                task_description: input["task_description"].as_str().unwrap_or("").to_string(),
                context: input["context"].as_str().map(|s| s.to_string()),
            },
            EventType::ContactsSync => EventPayload::ContactsSync { force: false },
            EventType::MessagesSync => EventPayload::MessagesSync { force: false },
            EventType::WhatsAppSync => EventPayload::WhatsAppSync {
                export_path: input["export_path"].as_str().map(|s| s.to_string()),
                force: false,
            },
            EventType::MorningBriefing => EventPayload::MorningBriefing {},
            EventType::EveningBriefing => EventPayload::EveningBriefing {},
            EventType::WeeklyDigest => EventPayload::WeeklyDigest {},
            EventType::VipEmailCheck => EventPayload::VipEmailCheck {
                email: input["email"].as_str().unwrap_or("").to_string(),
            },
            EventType::FollowUp => EventPayload::FollowUp {
                open_loop_id: input["open_loop_id"].as_str().unwrap_or("").to_string(),
                description: input["description"].as_str().unwrap_or("").to_string(),
            },
            EventType::RelationshipNudge => EventPayload::RelationshipNudge {
                email: input["email"].as_str().unwrap_or("").to_string(),
                name: input["name"].as_str().unwrap_or("").to_string(),
                last_contact_days: input["last_contact_days"].as_u64().unwrap_or(0) as u32,
            },
            EventType::WatchCheck => EventPayload::WatchCheck {
                subscription_id: input["subscription_id"].as_str().unwrap_or("").to_string(),
                topic: input["topic"].as_str().unwrap_or("").to_string(),
            },
            EventType::MessageInterruptCheck => EventPayload::MessageInterruptCheck {},
        };

        let id = self.sqlite_ref()?.enqueue_event(
            &event_type,
            &payload,
            scheduled_at.timestamp(),
            event_type.default_priority(),
            event_type.default_max_retries(),
            "chat",
            None,
        )?;
        Ok(format!(
            "Scheduled {} event with ID: {}",
            event_type_str, id
        ))
    }

    fn tool_cancel_event(&self, input: &serde_json::Value) -> Result<String> {
        let event_id = input["event_id"].as_str().unwrap_or("");
        self.sqlite_ref()?.cancel_event(event_id)?;
        Ok(format!("Cancelled event {}", event_id))
    }

    fn tool_list_events(&self, input: &serde_json::Value) -> Result<String> {
        let status = input["status"].as_str();
        let limit = input["limit"].as_i64().unwrap_or(10);
        let events = self.sqlite_ref()?.list_events(status, limit)?;
        if events.is_empty() {
            return Ok("No events found.".to_string());
        }
        let mut result = String::new();
        for e in &events {
            result.push_str(&format!(
                "- [{}] {} | {} | scheduled: {} | id: {}\n",
                e.status.as_str(),
                e.event_type.as_str(),
                e.error.as_deref().unwrap_or(""),
                e.scheduled_at.format("%Y-%m-%d %H:%M UTC"),
                e.id,
            ));
        }
        Ok(result)
    }

    fn tool_check_service_status(&self) -> Result<String> {
        let output = std::process::Command::new("launchctl")
            .args(["list"])
            .output()
            .context("failed to run launchctl")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let services: Vec<serde_json::Value> = stdout
            .lines()
            .filter(|line| line.contains("trusty-izzie"))
            .map(|line| {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                let pid = parts.first().copied().unwrap_or("-");
                let last_exit = parts.get(1).copied().unwrap_or("-");
                let label = parts.get(2).copied().unwrap_or("");
                let status = if pid == "-" { "stopped" } else { "running" };
                serde_json::json!({
                    "service": label,
                    "pid": pid,
                    "status": status,
                    "last_exit": last_exit,
                })
            })
            .collect();
        if services.is_empty() {
            return Ok(
                serde_json::json!([{"status": "no trusty-izzie services found"}]).to_string(),
            );
        }
        serde_json::to_string(&services).context("failed to serialize service status")
    }

    fn tool_get_version(&self) -> Result<String> {
        Ok(format!("trusty-izzie v{}", env!("CARGO_PKG_VERSION")))
    }

    fn tool_submit_github_issue(&self, input: &serde_json::Value) -> Result<String> {
        let title = input["title"].as_str().unwrap_or("").trim().to_string();
        let body = input["body"].as_str().unwrap_or("").trim().to_string();
        if title.is_empty() {
            return Ok("Error: title is required".to_string());
        }
        let mut cmd = std::process::Command::new("gh");
        cmd.args([
            "issue",
            "create",
            "--repo",
            "bobmatnyc/trusty-izzie",
            "--title",
            &title,
            "--body",
            &body,
        ]);
        if let Some(labels) = input["labels"].as_array() {
            for label in labels {
                if let Some(l) = label.as_str() {
                    cmd.args(["--label", l]);
                }
            }
        }
        let output = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("gh CLI not found — install from https://cli.github.com")
            } else {
                anyhow::anyhow!("failed to run gh: {}", e)
            }
        })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Ok(format!("Error filing issue: {}", stderr))
        }
    }

    fn tool_list_agents(&self) -> Result<String> {
        let mut agents = Vec::new();
        let entries = std::fs::read_dir(&self.agents_dir)
            .map_err(|e| anyhow::anyhow!("cannot read agents dir: {e}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let (model, max_runtime_mins, description, _body) =
                self.parse_agent_front_matter(&content);
            agents.push(serde_json::json!({
                "name": stem,
                "model": model,
                "description": description,
                "max_runtime_mins": max_runtime_mins,
            }));
        }
        serde_json::to_string(&agents).context("failed to serialize agents")
    }

    fn tool_run_agent(&self, input: &serde_json::Value) -> Result<String> {
        let agent_name = input["agent_name"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        let task_description = input["task_description"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if agent_name.is_empty() {
            return Ok("Error: agent_name is required".to_string());
        }
        if task_description.is_empty() {
            return Ok("Error: task_description is required".to_string());
        }
        let context = input["context"].as_str().map(|s| s.to_string());
        let payload = EventPayload::AgentRun {
            agent_name,
            task_description,
            context,
        };
        let now = chrono::Utc::now().timestamp();
        let id = self.sqlite_ref()?.enqueue_event(
            &EventType::AgentRun,
            &payload,
            now,
            EventType::AgentRun.default_priority(),
            EventType::AgentRun.default_max_retries(),
            "chat",
            None,
        )?;
        serde_json::to_string(&serde_json::json!({ "task_id": id }))
            .context("failed to serialize response")
    }

    fn tool_get_agent_task(&self, input: &serde_json::Value) -> Result<String> {
        let task_id = input["task_id"].as_str().unwrap_or("").trim().to_string();
        if task_id.is_empty() {
            return Ok("Error: task_id is required".to_string());
        }
        let task = self.sqlite_ref()?.get_agent_task(&task_id)?;
        match task {
            None => Ok(format!("No task found with id: {}", task_id)),
            Some(t) => {
                let output_preview = t.output.as_deref().map(|o| {
                    if o.len() > 500 {
                        format!("{}... [truncated]", &o[..500])
                    } else {
                        o.to_string()
                    }
                });
                serde_json::to_string(&serde_json::json!({
                    "id": t.id,
                    "agent_name": t.agent_name,
                    "task_description": t.task_description,
                    "status": t.status,
                    "model": t.model,
                    "output": output_preview,
                    "error": t.error,
                    "created_at": t.created_at,
                    "started_at": t.started_at,
                    "completed_at": t.completed_at,
                }))
                .context("failed to serialize task")
            }
        }
    }

    /// Parse YAML front-matter from agent MD content.
    /// Returns (model, max_runtime_mins, description, body).
    fn parse_agent_front_matter(&self, content: &str) -> (String, u32, String, String) {
        let default_model = "anthropic/claude-sonnet-4-5".to_string();
        if !content.starts_with("---") {
            return (default_model, 30, String::new(), content.to_string());
        }
        let rest = &content[3..];
        let end = rest.find("\n---").unwrap_or(rest.len());
        let front_matter = &rest[..end];
        let body = rest
            .get(end + 4..)
            .unwrap_or("")
            .trim_start_matches('\n')
            .to_string();
        let mut model = default_model;
        let mut max_runtime_mins: u32 = 30;
        let mut description = String::new();
        for line in front_matter.lines() {
            if let Some(val) = line.strip_prefix("model:") {
                model = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("max_runtime_mins:") {
                max_runtime_mins = val.trim().parse().unwrap_or(30);
            } else if let Some(val) = line.strip_prefix("description:") {
                description = val.trim().to_string();
            }
        }
        (model, max_runtime_mins, description, body)
    }

    fn tool_sync_contacts(&self) -> Result<String> {
        let sqlite = self.sqlite_ref()?;
        let now = chrono::Utc::now().timestamp();
        sqlite.enqueue_event(
            &EventType::ContactsSync,
            &EventPayload::ContactsSync { force: false },
            now,
            6,
            2,
            "chat",
            None,
        )?;
        Ok("macOS Contacts sync queued. I'll process your AddressBook and update my knowledge of your contacts shortly. Note: the first run will prompt for Contacts permission if not already granted.".to_string())
    }

    fn tool_sync_messages(&self) -> Result<String> {
        let sqlite = self.sqlite_ref()?;
        let now = chrono::Utc::now().timestamp();
        sqlite.enqueue_event(
            &EventType::MessagesSync,
            &EventPayload::MessagesSync { force: false },
            now,
            5,
            3,
            "chat",
            None,
        )?;
        Ok("iMessage/SMS sync queued. I'll read your Messages database and extract relationship context. This requires Full Disk Access — if not already granted, go to System Settings → Privacy & Security → Full Disk Access and add Trusty Izzie.".to_string())
    }

    fn tool_sync_whatsapp(&self, input: &serde_json::Value) -> Result<String> {
        let export_path = input["export_path"].as_str().map(|s| s.to_string());
        let sqlite = self.sqlite_ref()?;
        let now = chrono::Utc::now().timestamp();
        sqlite.enqueue_event(
            &EventType::WhatsAppSync,
            &EventPayload::WhatsAppSync {
                export_path,
                force: false,
            },
            now,
            5,
            3,
            "chat",
            None,
        )?;
        Ok("WhatsApp sync queued. I'll read your WhatsApp message history and extract relationship context.".to_string())
    }

    fn tool_search_imessages(&self, input: &serde_json::Value) -> Result<String> {
        let home = std::env::var("HOME").unwrap_or_default();
        let db_path = format!("{}/Library/Messages/chat.db", home);

        let contact = input["contact"].as_str().filter(|s| !s.is_empty());
        let query = input["query"].as_str().filter(|s| !s.is_empty());
        let limit = input["limit"].as_i64().unwrap_or(20).clamp(1, 50);
        let days_back = input["days_back"].as_i64().unwrap_or(30);
        let from_me = input["from_me"].as_bool();

        let conn = match Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(e) => {
                return Ok(format!(
                    "Error accessing iMessage database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let mut sql = String::from(
            "SELECT m.rowid, m.text, m.is_from_me, \
             datetime(m.date/1000000000 + 978307200, 'unixepoch') as sent_at, \
             h.id as contact \
             FROM message m \
             JOIN handle h ON m.handle_id = h.rowid \
             WHERE m.text IS NOT NULL \
             AND m.date/1000000000 + 978307200 > unixepoch() - ?*86400",
        );

        if from_me.is_some() {
            sql.push_str(" AND m.is_from_me = ?");
        }
        if contact.is_some() {
            sql.push_str(" AND h.id LIKE ?");
        }
        if query.is_some() {
            sql.push_str(" AND m.text LIKE ?");
        }
        sql.push_str(" ORDER BY m.date DESC LIMIT ?");

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(e) => {
                return Ok(format!(
                    "Error accessing iMessage database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        // Build params list dynamically.
        let contact_pattern = contact.map(|c| format!("%{}%", c));
        let query_pattern = query.map(|q| format!("%{}%", q));

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params.push(Box::new(days_back));
        if let Some(fm) = from_me {
            params.push(Box::new(fm as i32));
        }
        if let Some(ref cp) = contact_pattern {
            params.push(Box::new(cp.clone()));
        }
        if let Some(ref qp) = query_pattern {
            params.push(Box::new(qp.clone()));
        }
        params.push(Box::new(limit));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = match stmt.query(params_refs.as_slice()) {
            Ok(r) => r,
            Err(e) => {
                return Ok(format!(
                    "Error accessing iMessage database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let mut results = Vec::new();
        let mut rows = rows;
        while let Ok(Some(row)) = rows.next() {
            let rowid: i64 = row.get(0).unwrap_or(0);
            let text: String = row.get(1).unwrap_or_default();
            let is_from_me: i32 = row.get(2).unwrap_or(0);
            let sent_at: String = row.get(3).unwrap_or_default();
            let contact_id: String = row.get(4).unwrap_or_default();
            results.push(serde_json::json!({
                "rowid": rowid,
                "text": text,
                "is_from_me": is_from_me == 1,
                "sent_at": sent_at,
                "contact": contact_id,
            }));
        }

        serde_json::to_string(&results).context("failed to serialize iMessage results")
    }

    fn tool_search_contacts(&self, input: &serde_json::Value) -> Result<String> {
        let home = std::env::var("HOME").unwrap_or_default();
        let db_path = format!(
            "{}/Library/Application Support/AddressBook/AddressBook-v22.abcddb",
            home
        );

        let query = input["query"].as_str().unwrap_or("").trim();
        if query.is_empty() {
            return Ok("Error: query is required for search_contacts".to_string());
        }
        let limit = input["limit"].as_i64().unwrap_or(10).clamp(1, 100);
        let pattern = format!("%{}%", query);

        let conn = match Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(e) => {
                return Ok(format!(
                    "Error accessing Contacts database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let sql = "SELECT r.Z_PK, r.ZFIRSTNAME, r.ZLASTNAME, r.ZORGANIZATION, r.ZJOBTITLE, \
                   GROUP_CONCAT(DISTINCT p.ZFULLNUMBER) as phones, \
                   GROUP_CONCAT(DISTINCT e.ZADDRESS) as emails \
                   FROM ZABCDRECORD r \
                   LEFT JOIN ZABCDPHONENUMBER p ON p.ZOWNER = r.Z_PK \
                   LEFT JOIN ZABCDEMAILADDRESS e ON e.ZOWNER = r.Z_PK \
                   WHERE r.ZFIRSTNAME LIKE ?1 OR r.ZLASTNAME LIKE ?1 \
                      OR r.ZORGANIZATION LIKE ?1 \
                      OR p.ZFULLNUMBER LIKE ?1 \
                      OR e.ZADDRESS LIKE ?1 \
                   GROUP BY r.Z_PK \
                   LIMIT ?2";

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                return Ok(format!(
                    "Error accessing Contacts database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let rows = match stmt.query(rusqlite::params![pattern, limit]) {
            Ok(r) => r,
            Err(e) => {
                return Ok(format!(
                    "Error accessing Contacts database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let mut results = Vec::new();
        let mut rows = rows;
        while let Ok(Some(row)) = rows.next() {
            let first: Option<String> = row.get(1).ok();
            let last: Option<String> = row.get(2).ok();
            let org: Option<String> = row.get(3).ok();
            let job: Option<String> = row.get(4).ok();
            let phones_str: Option<String> = row.get(5).ok().flatten();
            let emails_str: Option<String> = row.get(6).ok().flatten();

            let name = match (first, last) {
                (Some(f), Some(l)) => format!("{} {}", f, l),
                (Some(f), None) => f,
                (None, Some(l)) => l,
                (None, None) => String::new(),
            };

            let phones: Vec<&str> = phones_str
                .as_deref()
                .map(|s| s.split(',').collect())
                .unwrap_or_default();
            let emails: Vec<&str> = emails_str
                .as_deref()
                .map(|s| s.split(',').collect())
                .unwrap_or_default();

            let mut entry = serde_json::json!({ "name": name });
            if let Some(o) = org {
                entry["organization"] = serde_json::Value::String(o);
            }
            if let Some(j) = job {
                entry["job_title"] = serde_json::Value::String(j);
            }
            if !phones.is_empty() {
                entry["phones"] = serde_json::json!(phones);
            }
            if !emails.is_empty() {
                entry["emails"] = serde_json::json!(emails);
            }
            results.push(entry);
        }

        serde_json::to_string(&results).context("failed to serialize contact results")
    }

    fn tool_search_whatsapp(&self, input: &serde_json::Value) -> Result<String> {
        let home = std::env::var("HOME").unwrap_or_default();
        let db_path = format!(
            "{}/Library/Group Containers/group.net.whatsapp.WhatsApp.shared/ChatStorage.sqlite",
            home
        );

        let contact = input["contact"].as_str().filter(|s| !s.is_empty());
        let query = input["query"].as_str().filter(|s| !s.is_empty());
        let limit = input["limit"].as_i64().unwrap_or(20).clamp(1, 50);
        let days_back = input["days_back"].as_i64().unwrap_or(30);

        let conn = match Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(e) => {
                return Ok(format!(
                    "Error accessing WhatsApp database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let mut sql = String::from(
            "SELECT m.ZTEXT as text, m.ZISFROMME as is_from_me, \
             datetime(m.ZMESSAGEDATE + 978307200, 'unixepoch') as sent_at, \
             s.ZPARTNERNAME as contact, s.ZCONTACTJID as jid, \
             s.ZSESSIONTYPE as chat_type \
             FROM ZWAMESSAGE m \
             JOIN ZWACHATSESSION s ON m.ZCHATSESSION = s.Z_PK \
             WHERE m.ZTEXT IS NOT NULL \
             AND m.ZMESSAGEDATE + 978307200 > unixepoch() - ?*86400",
        );

        if contact.is_some() {
            sql.push_str(" AND (s.ZPARTNERNAME LIKE ? OR s.ZCONTACTJID LIKE ?)");
        }
        if query.is_some() {
            sql.push_str(" AND m.ZTEXT LIKE ?");
        }
        sql.push_str(" ORDER BY m.ZMESSAGEDATE DESC LIMIT ?");

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(e) => {
                return Ok(format!(
                    "Error accessing WhatsApp database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let contact_pattern = contact.map(|c| format!("%{}%", c));
        let query_pattern = query.map(|q| format!("%{}%", q));

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params.push(Box::new(days_back));
        if let Some(ref cp) = contact_pattern {
            params.push(Box::new(cp.clone()));
            params.push(Box::new(cp.clone()));
        }
        if let Some(ref qp) = query_pattern {
            params.push(Box::new(qp.clone()));
        }
        params.push(Box::new(limit));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = match stmt.query(params_refs.as_slice()) {
            Ok(r) => r,
            Err(e) => {
                return Ok(format!(
                    "Error accessing WhatsApp database: {}. Make sure Izzie has Full Disk Access in System Settings > Privacy & Security.",
                    e
                ))
            }
        };

        let mut results = Vec::new();
        let mut rows = rows;
        while let Ok(Some(row)) = rows.next() {
            let text: String = row.get(0).unwrap_or_default();
            let is_from_me: i32 = row.get(1).unwrap_or(0);
            let sent_at: String = row.get(2).unwrap_or_default();
            let contact_name: String = row.get(3).unwrap_or_default();
            let jid: String = row.get(4).unwrap_or_default();
            let chat_type: i32 = row.get(5).unwrap_or(0);
            results.push(serde_json::json!({
                "text": text,
                "is_from_me": is_from_me == 1,
                "sent_at": sent_at,
                "contact": contact_name,
                "jid": jid,
                "chat_type": chat_type,
            }));
        }

        serde_json::to_string(&results).context("failed to serialize WhatsApp results")
    }

    fn tool_get_calendar_events(&self, input: &serde_json::Value) -> Result<String> {
        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let access_token = match self.get_valid_token(&primary_email) {
            Ok(t) => t,
            Err(_) => {
                // Fall back to kv_config for backward compat.
                match self.sqlite_ref()?.get_config("google_access_token")? {
                    Some(t) if !t.is_empty() => t,
                    _ => return Ok(
                        "I don't have a Google access token yet. Use /auth in Telegram or ask me to `add_account` to connect your Google account.".to_string()
                    ),
                }
            }
        };

        // Parse optional parameters.
        let days = input["days"].as_i64().unwrap_or(7).clamp(1, 30);
        let now = chrono::Utc::now();
        let time_min = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let time_max = (now + chrono::Duration::days(days))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        // Call Google Calendar API synchronously via a blocking runtime call.
        let rt = tokio::runtime::Handle::try_current();
        let events_json: serde_json::Value = if let Ok(handle) = rt {
            tokio::task::block_in_place(|| {
                handle.block_on(async {
                    let client = reqwest::Client::new();
                    let url = format!(
                        "https://www.googleapis.com/calendar/v3/calendars/primary/events\
                         ?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=20",
                        time_min, time_max
                    );
                    client
                        .get(&url)
                        .bearer_auth(&access_token)
                        .send()
                        .await
                        .map_err(|e| anyhow::anyhow!("Calendar API request failed: {e}"))?
                        .json::<serde_json::Value>()
                        .await
                        .map_err(|e| anyhow::anyhow!("Calendar API parse failed: {e}"))
                })
            })?
        } else {
            return Ok("Calendar lookup requires async runtime.".to_string());
        };

        // Check for auth errors.
        if let Some(err) = events_json.get("error") {
            let msg = err["message"].as_str().unwrap_or("unknown error");
            if err["code"].as_i64() == Some(401) {
                return Ok(format!(
                    "My Google access token has expired. Use /auth to reconnect. ({})",
                    msg
                ));
            }
            return Ok(format!("Google Calendar error: {}", msg));
        }

        let items = match events_json["items"].as_array() {
            Some(a) => a,
            None => return Ok("No calendar events found.".to_string()),
        };

        if items.is_empty() {
            return Ok(format!("No events in the next {} days.", days));
        }

        // Format events for the LLM.
        let mut lines = vec![format!("Upcoming events (next {} days):", days)];
        for item in items {
            let summary = item["summary"].as_str().unwrap_or("(no title)");
            let start = item["start"]["dateTime"]
                .as_str()
                .or_else(|| item["start"]["date"].as_str())
                .unwrap_or("unknown time");
            let location = item["location"].as_str().unwrap_or("");
            let attendee_count = item["attendees"].as_array().map(|a| a.len()).unwrap_or(0);

            let mut line = format!("• {} — {}", start, summary);
            if !location.is_empty() {
                line.push_str(&format!(" @ {}", location));
            }
            if attendee_count > 1 {
                line.push_str(&format!(" ({} attendees)", attendee_count));
            }
            lines.push(line);
        }
        Ok(lines.join("\n"))
    }

    fn tool_get_task_lists(&self) -> Result<String> {
        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let access_token = match self.get_valid_token(&primary_email) {
            Ok(t) => t,
            Err(e) => return Ok(format!("Cannot access Tasks: {e}")),
        };

        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| anyhow::anyhow!("No async runtime"))?;
        let lists: serde_json::Value = tokio::task::block_in_place(|| {
            handle.block_on(async {
                reqwest::Client::new()
                    .get("https://tasks.googleapis.com/tasks/v1/users/@me/lists")
                    .bearer_auth(&access_token)
                    .send()
                    .await?
                    .json::<serde_json::Value>()
                    .await
            })
        })?;

        if let Some(err) = lists.get("error") {
            return Ok(format!(
                "Tasks API error: {}",
                err["message"].as_str().unwrap_or("unknown")
            ));
        }

        let items = lists["items"].as_array();
        match items.map(|v| v.as_slice()) {
            None | Some([]) => Ok("No task lists found.".to_string()),
            Some(lists) => {
                let result: Vec<serde_json::Value> = lists
                    .iter()
                    .map(|l| {
                        serde_json::json!({
                            "id": l["id"].as_str().unwrap_or(""),
                            "title": l["title"].as_str().unwrap_or("(untitled)"),
                        })
                    })
                    .collect();
                serde_json::to_string(&result).context("failed to serialize task lists")
            }
        }
    }

    fn tool_get_tasks(&self, input: &serde_json::Value) -> Result<String> {
        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let access_token = match self.get_valid_token(&primary_email) {
            Ok(t) => t,
            Err(e) => return Ok(format!("Cannot access Tasks: {e}")),
        };

        // Optional: caller can specify a list ID; defaults to "@default".
        let list_id = input["list_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("@default");
        let show_completed = input["show_completed"].as_bool().unwrap_or(false);
        let max_results = input["max_results"].as_i64().unwrap_or(20).clamp(1, 100);

        let mut url = format!(
            "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks?maxResults={}",
            list_id, max_results
        );
        if !show_completed {
            url.push_str("&showCompleted=false&showHidden=false");
        }

        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| anyhow::anyhow!("No async runtime"))?;
        let resp: serde_json::Value = tokio::task::block_in_place(|| {
            handle.block_on(async {
                reqwest::Client::new()
                    .get(&url)
                    .bearer_auth(&access_token)
                    .send()
                    .await?
                    .json::<serde_json::Value>()
                    .await
            })
        })?;

        if let Some(err) = resp.get("error") {
            return Ok(format!(
                "Tasks API error: {}",
                err["message"].as_str().unwrap_or("unknown")
            ));
        }

        let items = resp["items"].as_array();
        match items.map(|v| v.as_slice()) {
            None | Some([]) => Ok(format!("No tasks found in list '{}'.", list_id)),
            Some(tasks) => {
                let mut lines = vec![format!("Tasks from '{}':", list_id)];
                for task in tasks {
                    let title = task["title"].as_str().unwrap_or("(untitled)");
                    let status = task["status"].as_str().unwrap_or("needsAction");
                    let due = task["due"].as_str().unwrap_or("");
                    let notes = task["notes"].as_str().unwrap_or("");
                    let status_icon = if status == "completed" { "✓" } else { "○" };
                    let mut line = format!("{} {}", status_icon, title);
                    if !due.is_empty() {
                        // Parse and reformat the due date (RFC3339)
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(due) {
                            line.push_str(&format!(" (due {})", dt.format("%b %d")));
                        }
                    }
                    if !notes.is_empty() {
                        line.push_str(&format!(" — {}", &notes[..notes.len().min(80)]));
                    }
                    lines.push(line);
                }
                Ok(lines.join("\n"))
            }
        }
    }

    fn tool_list_accounts(&self) -> Result<String> {
        let sqlite = self.sqlite_ref()?;
        let accounts = sqlite.list_accounts()?;
        if accounts.is_empty() {
            return Ok("No accounts registered yet.".to_string());
        }
        let json = serde_json::to_string(
            &accounts
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "email": a.email,
                        "type": a.account_type,
                        "active": a.is_active,
                        "display_name": a.display_name,
                    })
                })
                .collect::<Vec<_>>(),
        )?;
        Ok(json)
    }

    fn tool_add_account(&self) -> Result<String> {
        let sqlite = self.sqlite_ref()?;
        let (verifier, challenge) = trusty_email::auth::generate_pkce_pair();
        sqlite.set_config("oauth_pkce_verifier", &verifier)?;

        let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
        let ngrok_domain =
            std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
        let redirect_uri = format!("https://{}/api/auth/google/callback", ngrok_domain);

        let scopes = "https://mail.google.com/ https://www.googleapis.com/auth/userinfo.email";
        let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .context("failed to parse Google auth URL")?;
        url.query_pairs_mut()
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", scopes)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("access_type", "offline")
            .append_pair("prompt", "select_account consent");

        Ok(format!(
            "To add a Google account, visit:\n\n{}\n\nAfter granting access, the account will be registered automatically.",
            url
        ))
    }

    fn tool_remove_account(&self, input: &serde_json::Value) -> Result<String> {
        let email = input["email"].as_str().unwrap_or("").trim();
        if email.is_empty() {
            return Ok("Error: email is required".to_string());
        }
        let sqlite = self.sqlite_ref()?;
        match sqlite.deactivate_account(email) {
            Ok(()) => Ok(format!(
                "Account {} deactivated. It will no longer be synced.",
                email
            )),
            Err(e) => Ok(format!("Error: {e}")),
        }
    }

    fn tool_get_preferences(&self) -> Result<String> {
        let sqlite = self.sqlite_ref()?;
        let stored = sqlite.list_all_prefs()?;
        let defaults = [
            ("morning_briefing_enabled", "true"),
            ("evening_briefing_enabled", "true"),
            ("weekly_digest_enabled", "true"),
            ("relationship_nudge_enabled", "true"),
            ("open_loop_followup_enabled", "true"),
            ("watch_check_enabled", "true"),
            ("morning_briefing_time", "08:00"),
            ("evening_briefing_time", "18:00"),
            ("relationship_nudge_days", "21"),
            ("open_loop_followup_hours", "24"),
        ];
        let mut prefs = serde_json::Map::new();
        for (k, default) in &defaults {
            let val = stored
                .iter()
                .find(|(sk, _)| sk == k)
                .map(|(_, v)| v.as_str())
                .unwrap_or(default);
            prefs.insert(k.to_string(), serde_json::Value::String(val.to_string()));
        }
        serde_json::to_string(&prefs).context("failed to serialize prefs")
    }

    fn tool_set_preference(&self, input: &serde_json::Value) -> Result<String> {
        let key = input["key"].as_str().unwrap_or("").trim();
        let value = input["value"].as_str().unwrap_or("").trim();
        if key.is_empty() {
            return Ok("Error: key is required".to_string());
        }
        let valid_keys = [
            "morning_briefing_enabled",
            "evening_briefing_enabled",
            "weekly_digest_enabled",
            "relationship_nudge_enabled",
            "open_loop_followup_enabled",
            "watch_check_enabled",
            "morning_briefing_time",
            "evening_briefing_time",
            "relationship_nudge_days",
            "open_loop_followup_hours",
        ];
        if !valid_keys.contains(&key) {
            return Ok(format!(
                "Error: unknown preference key '{}'. Valid keys: {}",
                key,
                valid_keys.join(", ")
            ));
        }
        self.sqlite_ref()?.set_pref(key, value)?;
        Ok(format!("Preference '{}' set to '{}'.", key, value))
    }

    fn tool_add_vip_contact(&self, input: &serde_json::Value) -> Result<String> {
        let email = input["email"].as_str().unwrap_or("").trim();
        if email.is_empty() {
            return Ok("Error: email is required".to_string());
        }
        let name = input["name"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        self.sqlite_ref()?.upsert_vip_contact(email, name)?;
        Ok(format!("VIP contact '{}' added.", email))
    }

    fn tool_remove_vip_contact(&self, input: &serde_json::Value) -> Result<String> {
        let email = input["email"].as_str().unwrap_or("").trim();
        if email.is_empty() {
            return Ok("Error: email is required".to_string());
        }
        self.sqlite_ref()?.remove_vip_contact(email)?;
        Ok(format!("VIP contact '{}' removed.", email))
    }

    fn tool_list_vip_contacts(&self) -> Result<String> {
        let contacts = self.sqlite_ref()?.list_vip_contacts()?;
        if contacts.is_empty() {
            return Ok("No VIP contacts configured.".to_string());
        }
        let json: Vec<serde_json::Value> = contacts
            .into_iter()
            .map(|(email, name)| serde_json::json!({"email": email, "name": name}))
            .collect();
        serde_json::to_string(&json).context("failed to serialize contacts")
    }

    fn tool_add_watch_subscription(&self, input: &serde_json::Value) -> Result<String> {
        let topic = input["topic"].as_str().unwrap_or("").trim();
        if topic.is_empty() {
            return Ok("Error: topic is required".to_string());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let sqlite = self.sqlite_ref()?;
        sqlite.add_watch_subscription(&id, topic)?;
        // Enqueue an initial WatchCheck in 1 hour.
        let check_at = chrono::Utc::now().timestamp() + 3600;
        sqlite.enqueue_event(
            &EventType::WatchCheck,
            &EventPayload::WatchCheck {
                subscription_id: id.clone(),
                topic: topic.to_string(),
            },
            check_at,
            EventType::WatchCheck.default_priority(),
            EventType::WatchCheck.default_max_retries(),
            "chat",
            None,
        )?;
        Ok(format!(
            "Watch subscription added for '{}' (id: {}). First check in ~1 hour.",
            topic, id
        ))
    }

    fn tool_remove_watch_subscription(&self, input: &serde_json::Value) -> Result<String> {
        let id = input["id"].as_str().unwrap_or("").trim();
        if id.is_empty() {
            return Ok("Error: id is required".to_string());
        }
        self.sqlite_ref()?.remove_watch_subscription(id)?;
        Ok(format!("Watch subscription '{}' removed.", id))
    }

    fn tool_list_watch_subscriptions(&self) -> Result<String> {
        let subs = self.sqlite_ref()?.list_watch_subscriptions()?;
        if subs.is_empty() {
            return Ok("No active watch subscriptions.".to_string());
        }
        let json: Vec<serde_json::Value> = subs
            .into_iter()
            .map(|(id, topic)| serde_json::json!({"id": id, "topic": topic}))
            .collect();
        serde_json::to_string(&json).context("failed to serialize subscriptions")
    }

    fn tool_list_open_loops(&self) -> Result<String> {
        let loops = self.sqlite_ref()?.list_open_loops(Some("open"))?;
        if loops.is_empty() {
            return Ok("No open loops.".to_string());
        }
        let json: Vec<serde_json::Value> = loops
            .into_iter()
            .map(|l| {
                serde_json::json!({
                    "id": l.id,
                    "description": l.description,
                    "follow_up_at": l.follow_up_at,
                    "status": l.status,
                })
            })
            .collect();
        serde_json::to_string(&json).context("failed to serialize open loops")
    }

    fn tool_dismiss_open_loop(&self, input: &serde_json::Value) -> Result<String> {
        let id = input["id"].as_str().unwrap_or("").trim();
        if id.is_empty() {
            return Ok("Error: id is required".to_string());
        }
        self.sqlite_ref()?.close_open_loop(id, "dismissed")?;
        Ok(format!("Open loop '{}' dismissed.", id))
    }

    fn tool_execute_shell_command(&self, input: &serde_json::Value) -> Result<String> {
        let command = input["command"].as_str().unwrap_or("").trim();
        if command.is_empty() {
            return Ok("Error: 'command' field is required".to_string());
        }

        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to spawn command: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("[stderr] ");
            result.push_str(&stderr);
        }
        if result.is_empty() {
            result = format!("(exit code: {exit_code})");
        }

        if result.len() > 8000 {
            result.truncate(8000);
            result.push_str("\n...(truncated)");
        }

        Ok(result)
    }

    /// Process a single user turn, returning the assistant's reply.
    ///
    /// Implements a multi-turn JSON tool-calling loop:
    /// - LLM may return `toolCalls` to request tool execution.
    /// - Engine executes tools, appends results, and calls LLM again.
    /// - Loop runs until `toolCalls` is empty or `max_tool_iterations` is reached.
    pub async fn chat(
        &self,
        session: &mut ChatSession,
        user_message: &str,
    ) -> Result<StructuredResponse> {
        // 1. Append user message to session.
        session.messages.push(ChatMessage {
            id: Uuid::new_v4(),
            session_id: session.id,
            role: MessageRole::User,
            content: user_message.to_string(),
            tool_name: None,
            tool_result: None,
            token_count: None,
            created_at: chrono::Utc::now(),
        });

        // 2. Assemble RAG context.
        let ctx = self.context_assembler.assemble(user_message, "").await?;
        let context_section = self.context_assembler.render_context(&ctx);
        let now = chrono::Utc::now();
        let accounts_context = self.load_accounts_context();
        let system_content = system_prompt(now, &context_section, &accounts_context);

        // 3. Build the LLM message array from session history.
        let mut llm_messages: Vec<OrchatMessage> = vec![OrchatMessage {
            role: "system".to_string(),
            content: system_content,
        }];
        for msg in &session.messages {
            let role = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
                MessageRole::System => "system",
            };
            llm_messages.push(OrchatMessage {
                role: role.to_string(),
                content: msg.content.clone(),
            });
        }

        // 4. Tool call loop.
        let max_iters = (self.max_tool_iterations as usize).max(1);
        let mut structured = StructuredResponse {
            reply: String::new(),
            memories_to_save: vec![],
            referenced_entities: vec![],
            tool_calls: vec![],
        };
        let mut final_token_count: Option<u32> = None;
        let url = format!("{}/chat/completions", self.api_base);

        for _iteration in 0..max_iters {
            let request_body = OrchatRequest {
                model: &self.model,
                messages: llm_messages.clone(),
                tools: None,
                max_tokens: 2048,
                temperature: 0.7,
                response_format: ResponseFormat {
                    r#type: "json_object",
                },
            };

            let http_response = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&request_body)
                .send()
                .await
                .context("failed to send request to OpenRouter")?;

            let status = http_response.status();
            if !status.is_success() {
                let body = http_response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<unreadable>".to_string());
                return Err(anyhow::anyhow!("OpenRouter returned {}: {}", status, body));
            }

            let or_response: OrchatResponse = http_response
                .json()
                .await
                .context("failed to deserialize OpenRouter response")?;

            let raw_content = or_response
                .choices
                .into_iter()
                .next()
                .map(|c| c.message.content)
                .unwrap_or_default();

            final_token_count = or_response.usage.map(|u| u.total_tokens);
            structured = parse_response(&raw_content);
            structured.reply = clean_reply(&structured.reply);

            // Check for tool calls.
            let tool_calls = std::mem::take(&mut structured.tool_calls);
            if tool_calls.is_empty() {
                break; // No tool calls — final response.
            }

            // Execute each requested tool.
            let mut results_text = String::new();
            for tc in &tool_calls {
                let result = match serde_json::from_value::<ToolName>(serde_json::Value::String(
                    tc.name.clone(),
                )) {
                    Ok(tool_name) => self
                        .execute_tool(&tool_name, &tc.arguments)
                        .unwrap_or_else(|e| format!("Error: {e}")),
                    Err(_) => format!("Unknown tool: {}", tc.name),
                };
                tracing::debug!(tool = %tc.name, "tool executed");
                results_text.push_str(&format!("Tool `{}` returned:\n{}\n\n", tc.name, result));
            }

            // Append the assistant's tool-request turn and the results to the LLM context.
            llm_messages.push(OrchatMessage {
                role: "assistant".to_string(),
                content: raw_content,
            });
            llm_messages.push(OrchatMessage {
                role: "user".to_string(),
                content: format!(
                    "Tool results:\n\n{}Now please provide your final response.",
                    results_text
                ),
            });
        }

        if structured.reply.is_empty() {
            tracing::warn!(
                max_iters = max_iters,
                "tool call loop exhausted without producing a reply"
            );
            structured.reply =
                "I ran into an issue retrieving that information — please try again.".to_string();
        }

        // 5. Append the final assistant message to the persistent session.
        session.messages.push(ChatMessage {
            id: Uuid::new_v4(),
            session_id: session.id,
            role: MessageRole::Assistant,
            content: structured.reply.clone(),
            tool_name: None,
            tool_result: None,
            token_count: final_token_count,
            created_at: chrono::Utc::now(),
        });

        Ok(structured)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn system_prompt(now: chrono::DateTime<chrono::Utc>, context: &str, accounts_ctx: &str) -> String {
    let user_email =
        std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
    let user_name = std::env::var("TRUSTY_USER_NAME").unwrap_or_else(|_| "Masa".to_string());
    let context_section = if context.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", context)
    };
    let accounts_section = if accounts_ctx.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", accounts_ctx)
    };
    format!(
        r#"You are trusty-izzie, a personal AI assistant with deep knowledge of the user's professional relationships and work context. You run locally on the user's machine.

Today is {}. Current time: {}.

## About Your User
- **Name**: {user_name}
- **Email**: {user_email}
- **Timezone**: America/New_York (EDT, UTC-5)
- You are their personal assistant. Address them by name when appropriate. When they ask who they are or about themselves, use this information.
- **Location awareness**: When the user mentions being somewhere ("I'm in Berlin", "just landed in Tokyo", "heading to London"), treat it as their current location and save it as a memory with category "location". Surface this naturally when relevant — e.g. if they ask about weather, restaurants, or local contacts.
{context_section}{accounts_section}

## My Deployment

I am trusty-izzie v{}, running as macOS launchd services:
- Daemon (com.trusty-izzie.daemon) — event processing, Gmail sync
- API (com.trusty-izzie.api) — REST API on port 3456
- Telegram (com.trusty-izzie.telegram) — Telegram bot on port 3457

I can check my own service status with `check_service_status`, report my version with `get_version`, and file GitHub issues with `submit_github_issue`.

## What I Can Do
- **macOS Contacts**: I sync with your AddressBook via `sync_contacts`. I know your contact list.
- **Google Calendar**: I have access to your calendar via `get_calendar_events`. When asked about schedule, meetings, or upcoming events, I call this tool automatically. I can look ahead 1–30 days (default 7).
- **Google Tasks**: I can list task lists via `get_task_lists` and fetch tasks via `get_tasks`.

## Available Tools (complete list)
- `check_service_status` — report running status of all trusty-izzie launchd services
- `get_version` — return the current binary version
- `submit_github_issue` — file a GitHub issue via the `gh` CLI
- `schedule_event` — schedule a background task (email_sync, contacts_sync, memory_decay, reminder, agent_run, etc.)
- `cancel_event` — cancel a pending scheduled event by ID
- `list_events` — list scheduled or recent events, optionally filtered by status
- `run_agent` — enqueue a background research agent task
- `list_agents` — list available agent definitions
- `get_calendar_events` — fetch upcoming Google Calendar events (optional: days=1-30)
- `get_task_lists` — list the user's Google Task lists (uses bob@matsuoka.com)
- `get_tasks` — fetch tasks from a list (optional: list_id, max_results, show_completed; default: incomplete tasks from primary list)
- `get_agent_task` — get the status and output of an agent task by ID
- `list_accounts` — list connected Google accounts and their sync status
- `add_account` — add a new Google account (returns OAuth URL; run `/auth` in Telegram for full scope including Calendar and Tasks)
- `remove_account` — deactivate a secondary account
- `sync_contacts` — queue a macOS AddressBook contacts sync
- `execute_shell_command` — run a bash shell command on this Mac and return stdout/stderr
- `get_preferences` — view current proactive feature settings
- `set_preference` — toggle features on/off or adjust timing
- `add_vip_contact` / `remove_vip_contact` / `list_vip_contacts` — manage priority contacts
- `add_watch_subscription` / `remove_watch_subscription` / `list_watch_subscriptions` — monitor topics
- `list_open_loops` — see pending follow-ups
- `dismiss_open_loop` — dismiss a follow-up reminder
- `search_imessages`: Search iMessage history. Params: contact (string, partial match), query (keyword in text), limit (default 20), days_back (default 30), from_me (bool). Returns array of messages with contact, text, timestamp.
- `search_contacts`: Search macOS Address Book. Params: query (name/email/phone, required), limit (default 10). Returns contacts with name, phones, emails, organization.
- `search_whatsapp`: Search WhatsApp messages. Params: contact (string, partial match), query (keyword), limit (default 20), days_back (default 30). Returns messages with contact, text, timestamp.

## Proactive Features
I proactively send you briefings and updates. You can customize these:
- Use `get_preferences` to see current settings
- Use `set_preference` to toggle features on/off or adjust timing
- Use `add_vip_contact` / `list_vip_contacts` to manage priority contacts
- Use `add_watch_subscription` to monitor topics
- Use `list_open_loops` to see pending follow-ups

I do NOT have `read_file`, `write_file`, or `list_directory` tools. To access the file system, use `execute_shell_command` with commands like `ls`, `cat`, etc.

## Shell Access
I can run shell commands on your Mac via `execute_shell_command`. This lets me:
- Read/list files: `ls ~/Downloads`, `cat ~/some/file.txt`
- Run scripts: any bash command
- Check system state: `ps aux | grep something`, `df -h`, etc.
Use this for any scripting or file system tasks.

## Tool Calling Protocol

To invoke a tool, set `toolCalls` to a non-empty array. Leave `reply` empty when requesting tools — the user won't see it until you give your final response:

{{"reply":"","toolCalls":[{{"name":"get_calendar_events","arguments":{{"days":7}}}}],"memoriesToSave":[],"referencedEntities":[]}}

After tool results are injected into the conversation, give your final answer with `toolCalls` empty:

{{"reply":"Here is your schedule for the next week...","toolCalls":[],"memoriesToSave":[],"referencedEntities":[]}}

## Anti-Hallucination Rules

NEVER fabricate factual information. For these topics you MUST call the appropriate tool — never answer from memory or training data:
- Calendar / schedule / meetings → `get_calendar_events`
- Scheduled tasks / reminders / events → `list_events`
- Tasks / to-dos → `get_tasks` or `get_task_lists`
- Google accounts → `list_accounts`
- Service health / running processes → `check_service_status`
- Any file system, shell, or system state query → `execute_shell_command`
- User preferences → `get_preferences`
- Contact info (phone, email, address) → `search_contacts` ALWAYS before answering
- iMessage history → `search_imessages` ALWAYS; never fabricate message content
- WhatsApp history → `search_whatsapp` ALWAYS; never fabricate message content

If a tool returns no data (e.g. no calendar events), say so honestly. Never invent meetings, contacts, emails, or any factual data.

## CRITICAL OUTPUT FORMAT

Your ENTIRE response must be a single raw JSON object. Output ONLY the JSON — no prose before it, no explanation after it, no markdown code fences around it. Start your response with {{ and end with }}.

Required format (output this and nothing else):
{{"reply":"your response to the user (markdown allowed)","toolCalls":[],"memoriesToSave":[],"referencedEntities":[]}}

IMPORTANT: The "reply" field must ALWAYS be non-empty in your final response (when `toolCalls` is empty). Even for declarative statements, acknowledge receipt — e.g. "Got it, noted!" Never leave "reply" empty in a final response.

Be helpful, concise, and honest. Only include items in memoriesToSave if you learned something genuinely new and useful. Be selective — 0-1 memories per turn is typical."#,
        now.format("%A, %B %d, %Y"),
        now.format("%H:%M UTC"),
        env!("CARGO_PKG_VERSION"),
    )
}

/// Strip ```json ... ``` or ``` ... ``` fences if present.
fn strip_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    // Try ```json first, then plain ```
    for prefix in &["```json", "```"] {
        if let Some(after_open) = trimmed.strip_prefix(prefix) {
            if let Some(stripped) = after_open.strip_suffix("```") {
                return stripped.trim();
            }
        }
    }
    trimmed
}

fn parse_response(raw: &str) -> StructuredResponse {
    // 1. Try the whole response (after stripping outer fences)
    let cleaned = strip_fences(raw);
    if let Ok(s) = serde_json::from_str::<StructuredResponse>(cleaned) {
        return s;
    }

    // 2. Model sometimes emits text preamble then a ```json ... ``` block.
    //    Search for the LAST ```json block in the response.
    let mut last_fence_pos = None;
    let mut search_from = 0;
    while let Some(pos) = raw[search_from..].find("```json") {
        last_fence_pos = Some(search_from + pos);
        search_from += pos + 7; // 7 = len("```json")
    }
    if let Some(fence_start) = last_fence_pos {
        let after_open = &raw[fence_start + 7..]; // skip "```json"
        if let Some(close) = after_open.find("```") {
            let inner = after_open[..close].trim();
            if let Ok(s) = serde_json::from_str::<StructuredResponse>(inner) {
                return s;
            }
        }
    }

    // 3. Try to find any valid JSON object starting with '{' that has a "reply" field.
    let mut search = raw;
    while let Some(start) = search.find('{') {
        let candidate = &search[start..];
        if let Ok(s) = serde_json::from_str::<StructuredResponse>(candidate) {
            return s;
        }
        search = &search[start + 1..];
    }

    // 4. Fallback: treat the whole raw string as a plain-text reply.
    StructuredResponse {
        reply: raw.to_string(),
        memories_to_save: vec![],
        referenced_entities: vec![],
        tool_calls: vec![],
    }
}

/// Remove any trailing ```json ... ``` block that the model sometimes appends
/// inside the reply field as a "structured output" summary.
fn clean_reply(reply: &str) -> String {
    // Look for the LAST occurrence of ```json or ``` in the trimmed reply.
    // If found, strip from that point onward (and any trailing whitespace before it).
    let trimmed = reply.trim_end();
    for fence in &["```json", "```"] {
        if let Some(pos) = trimmed.rfind(fence) {
            // Only strip if the fence appears after a newline (not inline code)
            if pos == 0 || trimmed.as_bytes().get(pos - 1) == Some(&b'\n') {
                return trimmed[..pos].trim_end().to_string();
            }
        }
    }
    trimmed.to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_response_valid_json() {
        let json = r#"{"reply":"Hello there!","memoriesToSave":[],"referencedEntities":[]}"#;
        let result = parse_response(json);
        assert_eq!(result.reply, "Hello there!");
        assert!(result.memories_to_save.is_empty());
        assert!(result.referenced_entities.is_empty());
    }

    #[test]
    fn test_parse_response_fallback_on_bad_json() {
        let raw = "This is not JSON at all.";
        let result = parse_response(raw);
        assert_eq!(result.reply, raw);
        assert!(result.memories_to_save.is_empty());
    }

    #[test]
    fn test_parse_response_strips_markdown_fences() {
        let fenced =
            "```json\n{\"reply\":\"Hi!\",\"memoriesToSave\":[],\"referencedEntities\":[]}\n```";
        let result = parse_response(fenced);
        assert_eq!(result.reply, "Hi!");
    }

    #[test]
    fn test_system_prompt_contains_date() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(now, "", "");
        let year = now.format("%Y").to_string();
        assert!(prompt.contains(&year));
    }

    #[test]
    fn test_system_prompt_includes_context_when_nonempty() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(
            now,
            "## Relevant People & Projects\n- Alice (Person): alice",
            "",
        );
        assert!(prompt.contains("## Relevant People & Projects"));
    }

    #[test]
    fn test_system_prompt_no_context_section_when_empty() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(now, "", "");
        assert!(!prompt.contains("## Relevant"));
    }

    #[test]
    fn test_strip_fences_plain_backticks() {
        let fenced = "```\n{\"key\":\"value\"}\n```";
        let result = strip_fences(fenced);
        assert_eq!(result, "{\"key\":\"value\"}");
    }

    #[test]
    fn test_strip_fences_no_fences() {
        let raw = r#"{"key":"value"}"#;
        assert_eq!(strip_fences(raw), raw);
    }
}
