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

// Fallback when TRUSTY_PRIMARY_EMAIL env var is not set.
// Set TRUSTY_PRIMARY_EMAIL in your .env to your Google account email.
const PRIMARY_EMAIL: &str = "";

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
    /// Directory containing skill Markdown files (injected into system prompt).
    skills_dir: String,
    /// Dynamically registered skills (tool dispatch + system prompt contributions).
    skills: Vec<std::sync::Arc<dyn trusty_skill::Skill>>,
    /// Optional instance label injected into the system prompt (e.g. "DEV").
    instance_label: String,
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
            skills_dir: "docs/skills".to_string(),
            skills: vec![],
            instance_label: String::new(),
        }
    }

    /// Set the instance label (e.g. "DEV") shown in the system prompt.
    pub fn with_instance_label(mut self, label: String) -> Self {
        self.instance_label = label;
        self
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

    /// Set the skills directory for skill injection into the system prompt.
    pub fn with_skills_dir(mut self, skills_dir: String) -> Self {
        self.skills_dir = skills_dir;
        self
    }

    /// Register dynamically-dispatched skills with this engine.
    pub fn with_skills(mut self, skills: Vec<std::sync::Arc<dyn trusty_skill::Skill>>) -> Self {
        self.skills = skills;
        self
    }

    /// Dispatch a tool call by name — core tools first, then registered skills.
    pub async fn execute_tool_by_name(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<String> {
        // 1. Try core tools first.
        if let Ok(tool) =
            serde_json::from_value::<ToolName>(serde_json::Value::String(name.to_string()))
        {
            return self.execute_tool(&tool, args).await;
        }
        // 2. Try registered skills.
        let call = trusty_skill::SkillToolCall {
            name: name.to_string(),
            arguments: args.clone(),
        };
        for skill in &self.skills {
            if let Some(result) = skill.execute(&call).await {
                return result;
            }
        }
        // 3. Try per-turn script skills (hot-reloaded — picks up create_skill output).
        let script_skills =
            trusty_skill::load_script_skills(std::path::Path::new(&self.skills_dir));
        for skill in &script_skills {
            if let Some(result) = skill.execute(&call).await {
                return result;
            }
        }
        Err(anyhow::anyhow!("Unknown tool: {name}"))
    }

    /// Execute a chat tool call and return the result as a string.
    ///
    /// Returns an error string rather than propagating `Err` so the model can
    /// receive feedback about what went wrong.
    pub async fn execute_tool(&self, name: &ToolName, input: &serde_json::Value) -> Result<String> {
        match name {
            ToolName::ScheduleEvent => self.tool_schedule_event(input),
            ToolName::CancelEvent => self.tool_cancel_event(input),
            ToolName::ListEvents => self.tool_list_events(input),
            ToolName::CheckServiceStatus => self.tool_check_service_status().await,
            ToolName::GetVersion => self.tool_get_version(),
            ToolName::SubmitGithubIssue => self.tool_submit_github_issue(input).await,
            ToolName::ListAgents => self.tool_list_agents().await,
            ToolName::RunAgent => self.tool_run_agent(input),
            ToolName::GetAgentTask => self.tool_get_agent_task(input),
            ToolName::ListAccounts => self.tool_list_accounts(),
            ToolName::AddAccount => self.tool_add_account(),
            ToolName::RemoveAccount => self.tool_remove_account(input),
            ToolName::SyncContacts => self.tool_sync_contacts(),
            ToolName::SyncMessages => self.tool_sync_messages(),
            ToolName::SyncWhatsApp => self.tool_sync_whatsapp(input),
            ToolName::ExecuteShellCommand => self.tool_execute_shell_command(input).await,
            ToolName::GetCalendarEvents => self.tool_get_calendar_events(input).await,
            ToolName::CreateCalendarEvent => self.tool_create_calendar_event(input).await,
            ToolName::UpdateCalendarEvent => self.tool_update_calendar_event(input).await,
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
            ToolName::GetTaskLists => self.tool_get_task_lists(input).await,
            ToolName::GetTasks => self.tool_get_tasks(input).await,
            ToolName::GetTasksBulk => self.tool_get_tasks_bulk(input).await,
            ToolName::CompleteTask => self.tool_complete_task(input).await,
            ToolName::SearchImessages => self.tool_search_imessages(input),
            ToolName::SearchContacts => self.tool_search_contacts(input),
            ToolName::SearchWhatsapp => self.tool_search_whatsapp(input),
            ToolName::GetTrainSchedule => trusty_metro_north::get_train_schedule(input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),
            ToolName::GetTrainAlerts => trusty_metro_north::get_train_alerts(input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),
            ToolName::SearchSkills => {
                let query = input["query"].as_str().unwrap_or("");
                Ok(crate::skills::search_skills(query, &self.skills_dir))
            }
            ToolName::WebSearch => self.tool_web_search(input).await,
            ToolName::FetchPage => self.tool_fetch_page(input).await,
            ToolName::GetWeather => trusty_weather::get_weather(input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),
            ToolName::GetWeatherAlerts => trusty_weather::get_weather_alerts(input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),
            ToolName::CreateSkill => self.tool_create_skill(input).await,
            ToolName::SendEmail => self.tool_send_email(input).await,
            ToolName::ReplyEmail => self.tool_reply_email(input).await,
            ToolName::CreateTask => self.tool_create_task(input).await,
            ToolName::SearchSlack => self.tool_search_slack(input).await,
            ToolName::SearchAll => self.tool_search_all(input).await,
            ToolName::ListPendingActions => self.tool_list_pending_actions(),
            ToolName::ApproveAction => self.tool_approve_action(input).await,
            ToolName::RejectAction => self.tool_reject_action(input),
            ToolName::TavilySearch => self.tool_tavily_search(input).await,
            ToolName::FirecrawlScrape => self.tool_firecrawl_scrape(input).await,
            ToolName::SkyvernTask => self.tool_skyvern_task(input).await,
            ToolName::SerpApiSearch => self.tool_serpapi_search(input).await,
            ToolName::GetIzzieStatus => self.tool_get_status().await,
            _ => {
                tracing::warn!(tool = ?name, "tool called but not yet implemented");
                Ok("Tool not yet implemented.".to_string())
            }
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
    async fn get_valid_token(&self, user_id: &str) -> Result<String> {
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
        let client_secret = trusty_core::secrets::get("GOOGLE_CLIENT_SECRET").unwrap_or_default();
        let ngrok =
            std::env::var("TRUSTY_NGROK_DOMAIN").unwrap_or_else(|_| "izzie.ngrok.dev".to_string());
        let redirect_uri = format!("https://{}/api/auth/google/callback", ngrok);

        let auth = GoogleAuthClient::new(client_id, client_secret, redirect_uri);

        let new_token = auth.refresh_token(&refresh_token).await?;

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
            // Determine capabilities by checking whether a token row exists.
            // Accounts authorized with calendar scope will have an oauth_tokens row.
            // Primary always has full scope; secondary may have calendar if re-authed.
            let has_token = sqlite
                .get_access_token(&acc.email)
                .unwrap_or(None)
                .is_some();
            let capabilities = if acc.email == primary_email {
                "calendar, tasks, email, drive"
            } else if has_token {
                "calendar, tasks, email"
            } else {
                "email"
            };
            let identity_label = match acc.identity.as_str() {
                "work" => "work",
                _ => "personal",
            };
            lines.push(format!(
                "Account: {} | identity: {} | capabilities: {} | status: active",
                acc.email, identity_label, capabilities
            ));
        }
        lines.push(String::new());
        lines.push("To query a specific account's calendar, pass `account_email` to `get_calendar_events`.".to_string());
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
            EventType::TrainDelayCheck => EventPayload::TrainDelayCheck {},
            EventType::WeatherCheck => EventPayload::WeatherCheck {},
            EventType::StyleTraining => EventPayload::StyleTraining {},
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

    async fn tool_check_service_status(&self) -> Result<String> {
        let output = tokio::process::Command::new("launchctl")
            .args(["list"])
            .output()
            .await
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

    async fn tool_get_status(&self) -> Result<String> {
        use trusty_store::sqlite::StatusData;

        let skills_keys: &[&str] = &[
            "TAVILY_API_KEY",
            "BRAVE_SEARCH_API_KEY",
            "FIRECRAWL_API_KEY",
            "SERPAPI_API_KEY",
            "SKYVERN_API_KEY",
        ];
        let instance_env = std::env::var("TRUSTY_ENV").unwrap_or_else(|_| "prod".to_string());

        let data: StatusData = self
            .sqlite
            .as_deref()
            .and_then(|s| s.get_status_data(&instance_env, skills_keys).ok())
            .unwrap_or_else(|| StatusData {
                accounts: vec![],
                active_skills: vec![],
                entity_count: 0,
                memory_count: 0,
                instance_env: instance_env.clone(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                last_sync: vec![],
            });

        let mut lines: Vec<String> = vec![];
        lines.push(format!(
            "**Izzie v{}** ({})",
            data.version,
            data.instance_env.to_uppercase()
        ));
        lines.push(String::new());

        // Accounts
        lines.push("**Gmail Accounts:**".to_string());
        for acct in &data.accounts {
            let status = if acct.has_oauth_token {
                "connected"
            } else {
                "not authorized"
            };
            let sync = data
                .last_sync
                .iter()
                .find(|(e, _)| e == &acct.email)
                .map(|(_, t)| format!(", last sync: {}", t))
                .unwrap_or_default();
            lines.push(format!(
                "  {} ({}) — {}{}",
                acct.email, acct.account_type, status, sync
            ));
        }
        if data.accounts.is_empty() {
            lines.push("  No accounts configured".to_string());
        }
        lines.push(String::new());

        // Knowledge base
        lines.push(format!(
            "**Knowledge base:** {} entities, {} memories",
            data.entity_count, data.memory_count
        ));
        lines.push(String::new());

        // Skills
        lines.push("**Active skills:**".to_string());
        let skill_map: &[(&str, &str)] = &[
            ("TAVILY_API_KEY", "Tavily Search"),
            ("BRAVE_SEARCH_API_KEY", "Brave Search"),
            ("FIRECRAWL_API_KEY", "Firecrawl (Web Scrape)"),
            ("SERPAPI_API_KEY", "SerpApi (Google Search)"),
            ("SKYVERN_API_KEY", "Skyvern (Browser Automation)"),
        ];
        let active_names: Vec<&str> = skill_map
            .iter()
            .filter(|(key, _)| trusty_core::secrets::get(key).is_some())
            .map(|(_, name)| *name)
            .collect();
        if active_names.is_empty() {
            lines.push("  None configured".to_string());
        } else {
            for name in active_names {
                lines.push(format!("  + {}", name));
            }
        }
        lines.push(String::new());

        // Integrations
        lines.push("**Integrations:**".to_string());
        let slack_ok = std::env::var("SLACK_BOT_TOKEN").is_ok();
        let telegram_ok = self
            .sqlite
            .as_deref()
            .and_then(|s| s.get_config("telegram_bot_token").ok())
            .flatten()
            .is_some()
            || std::env::var("TELEGRAM_BOT_TOKEN").is_ok();
        let google_calendar = std::env::var("GOOGLE_CLIENT_ID").is_ok()
            && data.accounts.iter().any(|a| a.has_oauth_token);
        lines.push(format!(
            "  {} Telegram",
            if telegram_ok { "+" } else { "-" }
        ));
        lines.push(format!("  {} Slack", if slack_ok { "+" } else { "-" }));
        lines.push(format!(
            "  {} Google Calendar/Tasks",
            if google_calendar { "+" } else { "-" }
        ));

        Ok(lines.join("\n"))
    }

    async fn tool_submit_github_issue(&self, input: &serde_json::Value) -> Result<String> {
        let title = input["title"].as_str().unwrap_or("").trim().to_string();
        let body = input["body"].as_str().unwrap_or("").trim().to_string();
        if title.is_empty() {
            return Ok("Error: title is required".to_string());
        }
        let mut cmd = tokio::process::Command::new("gh");
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
        let output = cmd.output().await.map_err(|e| {
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

    async fn tool_list_agents(&self) -> Result<String> {
        let mut agents = Vec::new();
        let agents_dir = self.agents_dir.clone();
        let entries = tokio::task::spawn_blocking(move || std::fs::read_dir(&agents_dir))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {e}"))?
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
        let default_model = "anthropic/claude-sonnet-4.6".to_string();
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

    async fn tool_web_search(&self, input: &serde_json::Value) -> Result<String> {
        let api_key = trusty_core::secrets::get("BRAVE_SEARCH_API_KEY")
            .context("BRAVE_SEARCH_API_KEY not set")?;
        let query = input["query"]
            .as_str()
            .context("Missing required parameter: query")?;
        let count = input["count"].as_u64().unwrap_or(5).min(10) as u32;

        let url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
            urlencoding::encode(query),
            count
        );

        let response: serde_json::Value = reqwest::Client::new()
            .get(&url)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "gzip")
            .header("X-Subscription-Token", &api_key)
            .send()
            .await
            .context("Brave Search request failed")?
            .error_for_status()
            .context("Brave Search API error")?
            .json()
            .await
            .context("Failed to parse Brave Search response")?;

        let results = response["web"]["results"]
            .as_array()
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if results.is_empty() {
            return Ok(format!("No results found for: {query}"));
        }

        let mut lines = vec![format!("Search results for \"{query}\":\n")];
        for (i, result) in results.iter().enumerate() {
            let title = result["title"].as_str().unwrap_or("(no title)");
            let url = result["url"].as_str().unwrap_or("");
            let desc = result["description"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(200)
                .collect::<String>();
            lines.push(format!(
                "{}. **{}**\n   {}\n   {}\n",
                i + 1,
                title,
                desc,
                url
            ));
        }

        Ok(lines.join("\n"))
    }

    /// Fetch a web page and return its text content (HTML stripped).
    async fn tool_fetch_page(&self, input: &serde_json::Value) -> Result<String> {
        let url = input["url"]
            .as_str()
            .context("Missing required parameter: url")?;
        let max_chars = input["max_chars"].as_u64().unwrap_or(3000).min(8000) as usize;

        let response = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (compatible; TrustyIzzie/1.0)")
            .build()?
            .get(url)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .context("Failed to fetch page")?
            .error_for_status()
            .context("Page returned error status")?
            .text()
            .await
            .context("Failed to read page body")?;

        // Strip HTML tags with a simple regex-free approach
        let mut text = String::with_capacity(response.len());
        let mut in_tag = false;
        let mut in_script = false;
        let mut prev_was_space = false;
        let lower = response.to_lowercase();
        let bytes = response.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Skip <script>...</script> and <style>...</style> blocks
            if !in_tag
                && i + 7 <= lower.len()
                && (&lower[i..i + 7] == "<script" || &lower[i..i + 6] == "<style")
            {
                in_script = true;
            }
            if in_script {
                if i + 9 <= lower.len() && &lower[i..i + 9] == "</script>" {
                    i += 9;
                    in_script = false;
                    continue;
                }
                if i + 8 <= lower.len() && &lower[i..i + 8] == "</style>" {
                    i += 8;
                    in_script = false;
                    continue;
                }
                i += 1;
                continue;
            }
            match bytes[i] {
                b'<' => {
                    in_tag = true;
                    i += 1;
                }
                b'>' => {
                    in_tag = false;
                    // Tags act as whitespace
                    if !prev_was_space {
                        text.push(' ');
                        prev_was_space = true;
                    }
                    i += 1;
                }
                c if !in_tag => {
                    let ch = c as char;
                    if ch.is_whitespace() {
                        if !prev_was_space {
                            text.push(' ');
                            prev_was_space = true;
                        }
                    } else {
                        text.push(ch);
                        prev_was_space = false;
                    }
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }

        // Decode basic HTML entities
        let text = text
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "\'")
            .replace("&nbsp;", " ");

        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(format!("No readable content found at {url}"));
        }

        let excerpt: String = trimmed.chars().take(max_chars).collect();
        Ok(format!(
            "Content from {url}:\n\n{excerpt}{}",
            if trimmed.len() > max_chars {
                "\n\n[truncated]"
            } else {
                ""
            }
        ))
    }

    /// Search via Tavily AI-optimized search API.
    async fn tool_tavily_search(&self, input: &serde_json::Value) -> Result<String> {
        let api_key = match trusty_core::secrets::get("TAVILY_API_KEY") {
            Some(k) => k,
            None => return Ok("Tavily not configured (TAVILY_API_KEY missing)".to_string()),
        };
        let query = input["query"]
            .as_str()
            .context("Missing required parameter: query")?;

        let response: serde_json::Value = reqwest::Client::new()
            .post("https://api.tavily.com/search")
            .json(&serde_json::json!({
                "api_key": api_key,
                "query": query,
                "search_depth": "basic",
                "max_results": 5,
                "include_answer": true
            }))
            .send()
            .await
            .context("Tavily request failed")?
            .error_for_status()
            .context("Tavily API error")?
            .json()
            .await
            .context("Failed to parse Tavily response")?;

        let mut parts: Vec<String> = vec![];
        if let Some(answer) = response["answer"].as_str() {
            if !answer.is_empty() {
                parts.push(format!("**Answer**: {answer}\n"));
            }
        }
        if let Some(results) = response["results"].as_array() {
            for r in results {
                let title = r["title"].as_str().unwrap_or("(no title)");
                let url = r["url"].as_str().unwrap_or("");
                let snippet = r["content"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect::<String>();
                parts.push(format!("- [{title}]({url}): {snippet}"));
            }
        }
        if parts.is_empty() {
            Ok(format!("No results found for: {query}"))
        } else {
            Ok(parts.join("\n"))
        }
    }

    /// Scrape a URL to clean markdown via Firecrawl.
    async fn tool_firecrawl_scrape(&self, input: &serde_json::Value) -> Result<String> {
        let api_key = match trusty_core::secrets::get("FIRECRAWL_API_KEY") {
            Some(k) => k,
            None => return Ok("Firecrawl not configured (FIRECRAWL_API_KEY missing)".to_string()),
        };
        let url = input["url"]
            .as_str()
            .context("Missing required parameter: url")?;

        let response: serde_json::Value = reqwest::Client::new()
            .post("https://api.firecrawl.dev/v1/scrape")
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&serde_json::json!({
                "url": url,
                "formats": ["markdown"]
            }))
            .send()
            .await
            .context("Firecrawl request failed")?
            .error_for_status()
            .context("Firecrawl API error")?
            .json()
            .await
            .context("Failed to parse Firecrawl response")?;

        let markdown = response["data"]["markdown"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if markdown.is_empty() {
            return Ok(format!("No content extracted from {url}"));
        }
        const MAX_CHARS: usize = 4000;
        if markdown.len() > MAX_CHARS {
            let truncated: String = markdown.chars().take(MAX_CHARS).collect();
            Ok(format!("{truncated}\n\n[truncated — ask me to continue]"))
        } else {
            Ok(markdown)
        }
    }

    /// Run a browser automation task via Skyvern.
    async fn tool_skyvern_task(&self, input: &serde_json::Value) -> Result<String> {
        let api_key = match trusty_core::secrets::get("SKYVERN_API_KEY") {
            Some(k) => k,
            None => return Ok("Skyvern not configured (SKYVERN_API_KEY missing)".to_string()),
        };
        let url = input["url"]
            .as_str()
            .context("Missing required parameter: url")?;
        let goal = input["goal"]
            .as_str()
            .context("Missing required parameter: goal")?;
        let extract = input["extract"].as_str().unwrap_or("").to_string();

        let mut body = serde_json::json!({
            "url": url,
            "navigation_goal": goal,
            "proxy_location": "NONE"
        });
        if !extract.is_empty() {
            body["data_extraction_goal"] = serde_json::Value::String(extract);
        }

        let client = reqwest::Client::new();
        let create_resp: serde_json::Value = client
            .post("https://api.skyvern.com/api/v1/tasks")
            .header("x-api-key", &api_key)
            .json(&body)
            .send()
            .await
            .context("Skyvern create task request failed")?
            .error_for_status()
            .context("Skyvern API error on task creation")?
            .json()
            .await
            .context("Failed to parse Skyvern create response")?;

        let task_id = create_resp["task_id"]
            .as_str()
            .context("Skyvern response missing task_id")?
            .to_string();

        // Poll up to 30 times with 2s sleep (60s total timeout).
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let poll: serde_json::Value = client
                .get(format!("https://api.skyvern.com/api/v1/tasks/{task_id}"))
                .header("x-api-key", &api_key)
                .send()
                .await
                .context("Skyvern poll request failed")?
                .error_for_status()
                .context("Skyvern API error on poll")?
                .json()
                .await
                .context("Failed to parse Skyvern poll response")?;

            let status = poll["status"].as_str().unwrap_or("unknown");
            match status {
                "completed" => {
                    let info = poll["extracted_information"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    if info.is_empty() {
                        return Ok(format!("Task {task_id} completed with no extracted data."));
                    }
                    const MAX_CHARS: usize = 3000;
                    return if info.len() > MAX_CHARS {
                        let truncated: String = info.chars().take(MAX_CHARS).collect();
                        Ok(format!("{truncated}\n\n[truncated — ask me to continue]"))
                    } else {
                        Ok(info)
                    };
                }
                "failed" => {
                    let reason = poll["failure_reason"].as_str().unwrap_or("unknown reason");
                    return Ok(format!("Skyvern task failed: {reason}"));
                }
                _ => {} // "created" | "running" — keep polling
            }
        }

        Ok(format!(
            "Task running at https://app.skyvern.com/tasks/{task_id} — check back shortly"
        ))
    }

    /// Search Google via SerpApi and return structured results.
    async fn tool_serpapi_search(&self, input: &serde_json::Value) -> Result<String> {
        let api_key = match trusty_core::secrets::get("SERPAPI_API_KEY") {
            Some(k) => k,
            None => return Ok("SerpApi not configured (SERPAPI_API_KEY missing)".to_string()),
        };
        let query = input["query"]
            .as_str()
            .context("Missing required parameter: query")?;
        let engine = input["engine"].as_str().unwrap_or("google");

        let response: serde_json::Value = self
            .http
            .get("https://serpapi.com/search")
            .query(&[
                ("api_key", api_key.as_str()),
                ("q", query),
                ("engine", engine),
                ("num", "5"),
            ])
            .send()
            .await
            .context("SerpApi request failed")?
            .error_for_status()
            .context("SerpApi API error")?
            .json()
            .await
            .context("Failed to parse SerpApi response")?;

        let mut parts: Vec<String> = vec![];

        // Lead with direct answer box if present.
        if let Some(answer) = response["answer_box"]["answer"]
            .as_str()
            .or_else(|| response["answer_box"]["snippet"].as_str())
        {
            if !answer.is_empty() {
                parts.push(format!("**Answer**: {answer}\n"));
            }
        }

        // Knowledge graph description.
        if let Some(kg) = response["knowledge_graph"]["description"].as_str() {
            if !kg.is_empty() {
                parts.push(format!("**Knowledge Graph**: {kg}\n"));
            }
        }

        // Organic results.
        if let Some(results) = response["organic_results"].as_array() {
            for r in results {
                let title = r["title"].as_str().unwrap_or("(no title)");
                let link = r["link"].as_str().unwrap_or("");
                let snippet = r["snippet"].as_str().unwrap_or("");
                parts.push(format!("- [{title}]({link}): {snippet}"));
            }
        }

        if parts.is_empty() {
            return Ok(format!("No results found for: {query}"));
        }

        let output = parts.join("\n");
        const MAX_CHARS: usize = 3000;
        if output.len() > MAX_CHARS {
            let truncated: String = output.chars().take(MAX_CHARS).collect();
            Ok(format!("{truncated}\n\n[truncated]"))
        } else {
            Ok(output)
        }
    }

    async fn fetch_calendar_events_for(&self, email: &str, days: i64) -> Result<Vec<String>> {
        let access_token = self.get_valid_token(email).await?;

        let now = chrono::Utc::now();
        let time_min = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let time_max = (now + chrono::Duration::days(days))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events\
             ?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=20",
            time_min, time_max
        );
        let events_json: serde_json::Value = self
            .http
            .get(&url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Calendar API request failed: {e}"))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| anyhow::anyhow!("Calendar API parse failed: {e}"))?;

        if events_json.get("error").is_some() {
            return Ok(vec![]);
        }

        let items = match events_json["items"].as_array() {
            Some(a) if !a.is_empty() => a,
            _ => return Ok(vec![]),
        };

        let mut lines = Vec::new();
        for item in items {
            let summary = item["summary"].as_str().unwrap_or("(no title)");
            let start = item["start"]["dateTime"]
                .as_str()
                .or_else(|| item["start"]["date"].as_str())
                .unwrap_or("unknown time");
            let location = item["location"].as_str().unwrap_or("");
            let attendee_count = item["attendees"].as_array().map(|a| a.len()).unwrap_or(0);

            let event_id = item["id"].as_str().unwrap_or("");
            let mut line = format!("• {} — {}", start, summary);
            if !location.is_empty() {
                line.push_str(&format!(" @ {}", location));
            }
            if attendee_count > 1 {
                line.push_str(&format!(" ({} attendees)", attendee_count));
            }
            if !event_id.is_empty() {
                // Include IDs so update_calendar_event can be called without extra lookup.
                // calendar_id = email (Google accepts email as the calendarId for primary).
                line.push_str(&format!(
                    " [event_id: {}, calendar_id: {}]",
                    event_id, email
                ));
            }
            lines.push(line);
        }
        Ok(lines)
    }

    async fn tool_get_calendar_events(&self, input: &serde_json::Value) -> Result<String> {
        let account_email = input["account_email"].as_str().filter(|e| !e.is_empty());
        let days = input["days"].as_i64().unwrap_or(7).clamp(1, 30);

        if let Some(email) = account_email {
            if self.get_valid_token(email).await.is_err() {
                return Ok(format!(
                    "No calendar access for {}. The account may not have been authorized with calendar scope. Try re-authorizing with /auth.",
                    email
                ));
            }
            let lines = self.fetch_calendar_events_for(email, days).await?;
            if lines.is_empty() {
                return Ok(format!("No events in the next {} days.", days));
            }
            let mut out = vec![format!("Upcoming events (next {} days):", days)];
            out.extend(lines);
            return Ok(out.join("\n"));
        }

        // No account specified — query all accounts with valid tokens.
        let accounts = self.sqlite_ref()?.list_accounts()?;
        let mut all_sections = Vec::new();
        for acc in &accounts {
            match self.fetch_calendar_events_for(&acc.email, days).await {
                Ok(lines) if !lines.is_empty() => {
                    let label = if acc.identity == "work" || acc.email.contains("duettoresearch") {
                        "Work calendar"
                    } else if acc.email == "bob@matsuoka.com" {
                        "Personal calendar"
                    } else {
                        &acc.email
                    };
                    all_sections.push(format!(
                        "**{}** ({}):\n{}",
                        label,
                        acc.email,
                        lines.join("\n")
                    ));
                }
                _ => {}
            }
        }
        if all_sections.is_empty() {
            return Ok(format!(
                "No events in the next {} days across all accounts.",
                days
            ));
        }
        Ok(all_sections.join("\n\n"))
    }

    async fn tool_create_calendar_event(&self, input: &serde_json::Value) -> Result<String> {
        let account_email = input["account_email"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if account_email.is_empty() {
            return Ok("Failed to create event: account_email is required".to_string());
        }
        let title = input["title"].as_str().unwrap_or("").trim().to_string();
        if title.is_empty() {
            return Ok("Failed to create event: title is required".to_string());
        }
        let start_datetime = input["start_datetime"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        let end_datetime = input["end_datetime"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if start_datetime.is_empty() || end_datetime.is_empty() {
            return Ok(
                "Failed to create event: start_datetime and end_datetime are required".to_string(),
            );
        }

        let access_token = match self.get_valid_token(&account_email).await {
            Ok(t) => t,
            Err(e) => {
                return Ok(format!(
                    "Failed to create event: no calendar access for {} — {}",
                    account_email, e
                ))
            }
        };

        let mut body = serde_json::json!({
            "summary": title,
            "start": {"dateTime": start_datetime, "timeZone": "America/New_York"},
            "end":   {"dateTime": end_datetime,   "timeZone": "America/New_York"},
        });

        if let Some(desc) = input["description"].as_str().filter(|s| !s.is_empty()) {
            body["description"] = serde_json::Value::String(desc.to_string());
        }

        if let Some(attendees) = input["attendees"].as_array() {
            let list: Vec<serde_json::Value> = attendees
                .iter()
                .filter_map(|v| v.as_str())
                .map(|email| serde_json::json!({"email": email}))
                .collect();
            if !list.is_empty() {
                body["attendees"] = serde_json::Value::Array(list);
            }
        }

        let resp: serde_json::Value = match self
            .http
            .post("https://www.googleapis.com/calendar/v3/calendars/primary/events")
            .bearer_auth(&access_token)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => match r.json::<serde_json::Value>().await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(format!(
                        "Failed to create event: could not parse API response — {}",
                        e
                    ))
                }
            },
            Err(e) => {
                return Ok(format!(
                    "Failed to create event: API request failed — {}",
                    e
                ))
            }
        };

        if let Some(err) = resp.get("error") {
            let msg = err["message"].as_str().unwrap_or("unknown error");
            return Ok(format!(
                "Failed to create event: Google API error — {}",
                msg
            ));
        }

        let event_id = match resp["id"].as_str() {
            Some(id) if !id.is_empty() => id,
            _ => {
                return Ok(format!(
                    "Action UNCONFIRMED: calendar API returned no event ID for '{}'. \
                     The event may NOT have been created. Raw response: {}",
                    title,
                    serde_json::to_string(&resp)
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect::<String>()
                ))
            }
        };
        let html_link = resp["htmlLink"].as_str().unwrap_or("");
        Ok(format!(
            "Event created (confirmed id: {}): '{}' starting {}. Link: {}",
            event_id, title, start_datetime, html_link
        ))
    }

    async fn tool_update_calendar_event(&self, input: &serde_json::Value) -> Result<String> {
        let account_email = input["account_email"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if account_email.is_empty() {
            return Ok("Failed to update event: account_email is required".to_string());
        }
        let calendar_id = input["calendar_id"]
            .as_str()
            .unwrap_or("primary")
            .trim()
            .to_string();
        let event_id = input["event_id"].as_str().unwrap_or("").trim().to_string();
        if event_id.is_empty() {
            return Ok("Failed to update event: event_id is required".to_string());
        }

        let access_token = match self.get_valid_token(&account_email).await {
            Ok(t) => t,
            Err(e) => {
                return Ok(format!(
                    "Failed to update event: no calendar access for {} — {}",
                    account_email, e
                ))
            }
        };

        // Build the PATCH body with only the fields that were provided.
        let mut body = serde_json::json!({});
        let mut changed: Vec<String> = vec![];

        if let Some(summary) = input["summary"].as_str().filter(|s| !s.is_empty()) {
            body["summary"] = serde_json::Value::String(summary.to_string());
            changed.push(format!("Title: {}", summary));
        }
        if let Some(start) = input["start_time"].as_str().filter(|s| !s.is_empty()) {
            body["start"] = serde_json::json!({"dateTime": start, "timeZone": "America/New_York"});
            changed.push(format!("Start: {}", start));
        }
        if let Some(end) = input["end_time"].as_str().filter(|s| !s.is_empty()) {
            body["end"] = serde_json::json!({"dateTime": end, "timeZone": "America/New_York"});
            changed.push(format!("End: {}", end));
        }
        if let Some(desc) = input["description"].as_str().filter(|s| !s.is_empty()) {
            body["description"] = serde_json::Value::String(desc.to_string());
            changed.push(format!("Description: {}", desc));
        }
        if let Some(loc) = input["location"].as_str().filter(|s| !s.is_empty()) {
            body["location"] = serde_json::Value::String(loc.to_string());
            changed.push(format!("Location: {}", loc));
        }

        if body.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            return Ok(
                "Nothing to update: provide at least one of summary, start_time, end_time, description, or location".to_string(),
            );
        }

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
            calendar_id, event_id
        );
        let resp: serde_json::Value = match self
            .http
            .patch(&url)
            .bearer_auth(&access_token)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => match r.json::<serde_json::Value>().await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(format!(
                        "Failed to update event: could not parse API response — {}",
                        e
                    ))
                }
            },
            Err(e) => {
                return Ok(format!(
                    "Failed to update event: API request failed — {}",
                    e
                ))
            }
        };

        if let Some(err) = resp.get("error") {
            let msg = err["message"].as_str().unwrap_or("unknown error");
            return Ok(format!(
                "Failed to update event: Google API error — {}",
                msg
            ));
        }

        match resp["id"].as_str() {
            Some(id) if !id.is_empty() => {
                let mut out = vec![format!("CONFIRMED update_calendar_event id={}", id)];
                out.extend(changed);
                Ok(out.join("\n"))
            }
            _ => Ok(format!(
                "Action UNCONFIRMED: calendar API returned no event ID for update of '{}'. \
                 The event may NOT have been updated. Raw response: {}",
                event_id,
                serde_json::to_string(&resp)
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect::<String>()
            )),
        }
    }

    async fn tool_get_task_lists(&self, input: &serde_json::Value) -> Result<String> {
        let account_email = input["account_email"].as_str().filter(|e| !e.is_empty());
        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = account_email.unwrap_or(&primary_email);
        let access_token = match self.get_valid_token(email).await {
            Ok(t) => t,
            Err(e) => return Ok(format!("Cannot access Tasks for {email}: {e}")),
        };

        let lists: serde_json::Value = self
            .http
            .get("https://tasks.googleapis.com/tasks/v1/users/@me/lists")
            .bearer_auth(&access_token)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(err) = lists.get("error") {
            return Ok(format!(
                "Tasks API error: {}",
                err["message"].as_str().unwrap_or("unknown")
            ));
        }

        let items = lists["items"].as_array();
        match items.map(|v| v.as_slice()) {
            None | Some([]) => Ok(format!("No task lists found for {email}.")),
            Some(lists) => {
                let result: Vec<serde_json::Value> = lists
                    .iter()
                    .map(|l| {
                        serde_json::json!({
                            "account": email,
                            "id": l["id"].as_str().unwrap_or(""),
                            "title": l["title"].as_str().unwrap_or("(untitled)"),
                        })
                    })
                    .collect();
                serde_json::to_string(&result).context("failed to serialize task lists")
            }
        }
    }

    async fn tool_get_tasks(&self, input: &serde_json::Value) -> Result<String> {
        let account_email = input["account_email"].as_str().filter(|e| !e.is_empty());
        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = account_email.unwrap_or(&primary_email);
        let access_token = match self.get_valid_token(email).await {
            Ok(t) => t,
            Err(e) => return Ok(format!("Cannot access Tasks for {email}: {e}")),
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

        let resp: serde_json::Value = self
            .http
            .get(&url)
            .bearer_auth(&access_token)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

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
                        let notes_preview: String = notes.chars().take(80).collect();
                        line.push_str(&format!(" — {}", &notes_preview));
                    }
                    lines.push(line);
                }
                Ok(lines.join("\n"))
            }
        }
    }

    async fn tool_get_tasks_bulk(&self, input: &serde_json::Value) -> Result<String> {
        let account_email = input["account_email"].as_str().filter(|e| !e.is_empty());
        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = account_email.unwrap_or(&primary_email);
        let access_token = match self.get_valid_token(email).await {
            Ok(t) => t,
            Err(e) => return Ok(format!("Cannot access Tasks for {email}: {e}")),
        };

        // Step 1: fetch all task lists.
        let lists_resp: serde_json::Value = self
            .http
            .get("https://tasks.googleapis.com/tasks/v1/users/@me/lists")
            .bearer_auth(&access_token)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(err) = lists_resp.get("error") {
            return Ok(format!(
                "Tasks API error: {}",
                err["message"].as_str().unwrap_or("unknown")
            ));
        }

        let list_items = match lists_resp["items"].as_array() {
            None => return Ok(format!("No task lists found for {email}.")),
            Some(items) if items.is_empty() => {
                return Ok(format!("No task lists found for {email}."));
            }
            Some(items) => items.clone(),
        };

        // Step 2: for each list, fetch incomplete tasks and format output.
        let mut output_sections: Vec<String> = Vec::new();
        for list in &list_items {
            let list_id = list["id"].as_str().unwrap_or("@default");
            let list_title = list["title"].as_str().unwrap_or("(untitled)");

            let url = format!(
                "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks?maxResults=100&showCompleted=false&showHidden=false",
                list_id
            );
            let tasks_resp: serde_json::Value = self
                .http
                .get(&url)
                .bearer_auth(&access_token)
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if tasks_resp.get("error").is_some() {
                continue; // skip lists we can't read
            }

            let tasks = match tasks_resp["items"].as_array() {
                None => continue,
                Some(t) if t.is_empty() => continue,
                Some(t) => t.clone(),
            };

            let mut lines = vec![format!("**{}** (account: {})", list_title, email)];
            for task in &tasks {
                let title = task["title"].as_str().unwrap_or("(untitled)");
                let due = task["due"].as_str().unwrap_or("");
                let notes = task["notes"].as_str().unwrap_or("");
                let mut line = format!("- [ ] {}", title);
                if !due.is_empty() {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(due) {
                        line.push_str(&format!(" (due: {})", dt.format("%b %d")));
                    }
                }
                if !notes.is_empty() {
                    let notes_preview: String = notes.chars().take(80).collect();
                    line.push_str(&format!(" — {}", &notes_preview));
                }
                lines.push(line);
            }
            output_sections.push(lines.join("\n"));
        }

        if output_sections.is_empty() {
            Ok(format!("No incomplete tasks found for {email}."))
        } else {
            Ok(output_sections.join("\n\n"))
        }
    }

    async fn tool_complete_task(&self, input: &serde_json::Value) -> Result<String> {
        let account_email = input["account_email"].as_str().filter(|e| !e.is_empty());
        let primary_email =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = account_email.unwrap_or(&primary_email);
        let access_token = match self.get_valid_token(email).await {
            Ok(t) => t,
            Err(e) => return Ok(format!("Cannot access Tasks for {email}: {e}")),
        };

        let task_list_id = match input["task_list_id"].as_str().filter(|s| !s.is_empty()) {
            Some(id) => id,
            None => return Ok("Missing required parameter: task_list_id".to_string()),
        };
        let task_id = match input["task_id"].as_str().filter(|s| !s.is_empty()) {
            Some(id) => id,
            None => return Ok("Missing required parameter: task_id".to_string()),
        };

        let url = format!(
            "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks/{}",
            task_list_id, task_id
        );
        let resp = self
            .http
            .patch(&url)
            .bearer_auth(&access_token)
            .json(&serde_json::json!({"status": "completed"}))
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            Ok("Task marked complete.".to_string())
        } else {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            Ok(format!(
                "Failed to complete task (HTTP {}): {}",
                status, msg
            ))
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
                        "identity": a.identity,
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

        let scopes = "https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/tasks https://www.googleapis.com/auth/drive.readonly https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";
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

    // ── Skill builder ─────────────────────────────────────────────────────────

    /// Make a single LLM completion call (no tools, no session).
    /// Used internally for sub-tasks like skill design and code generation.
    async fn llm_sub_call(&self, model: &str, system: &str, user: &str) -> anyhow::Result<String> {
        let url = format!("{}/chat/completions", self.api_base);
        let body = serde_json::json!({
            "model": model,
            "max_tokens": 4096,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        });
        let resp: serde_json::Value = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://trusty-izzie")
            .header("X-Title", "trusty-izzie skill builder")
            .json(&body)
            .send()
            .await
            .context("skill builder LLM request failed")?
            .json()
            .await
            .context("failed to parse skill builder LLM response")?;
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .context("no content in skill builder response")?
            .to_string();
        Ok(content)
    }

    async fn tool_create_skill(&self, input: &serde_json::Value) -> Result<String> {
        let name = input["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: name"))?
            .to_lowercase()
            .replace(' ', "-");
        let description = input["description"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: description"))?;

        tracing::info!(skill = %name, "create_skill: designing with Opus");

        // ── Step 1: Opus designs the skill spec ──────────────────────────────
        let design_system = r#"You are a skill architect for a personal AI assistant called Izzie.
Design a skill specification in YAML frontmatter format for a new skill.

The frontmatter MUST follow this exact schema:
---
name: <kebab-case-name>
version: "1.0"
description: "<one sentence>"
execute:
  runtime: python3
  command: scripts/skills/<name>.py
  arg_format: json-stdin
tools:
  - name: <tool_name>
    description: "<what it does>"
    parameters:
      type: object
      properties:
        <param>: { type: string, description: "<desc>" }
      required: [<param>]
examples:
  - user: "<example user message>"
    tool: <tool_name>
    args: { <param>: "<value>" }
---

After the frontmatter, write 2-3 paragraphs of markdown explaining:
- When to use this skill
- What data sources or APIs it uses
- Any important limitations or usage notes

Output ONLY the complete .md file content (frontmatter + markdown body). No preamble."#;

        let design_user =
            format!("Design a skill named '{name}' that does the following:\n\n{description}");

        let skill_md = self
            .llm_sub_call("anthropic/claude-opus-4-5", design_system, &design_user)
            .await?;

        // ── Step 2: Sonnet writes the Python implementation ───────────────────
        tracing::info!(skill = %name, "create_skill: implementing with Sonnet");

        let impl_system = r#"You are a Python developer writing a skill script for a personal AI assistant.

The script will be called via subprocess with tool arguments as JSON on stdin.
Read JSON from stdin, execute the requested tool, print result to stdout, exit 0.
On error, print error message to stdout (not stderr) and exit 0.

Requirements:
- Read JSON from stdin: `import json, sys; args = json.load(sys.stdin)`
- The JSON has: {"name": "<tool_name>", "arguments": {<tool_args>}}
- Dispatch on args["name"] to call the right function
- Print plain text result to stdout
- Use only stdlib + requests (no other deps assumed)
- Handle errors gracefully — never crash with an uncaught exception
- Keep it simple and readable

Output ONLY the Python code, no markdown fences, no explanation."#;

        let impl_user = format!("Write the Python implementation for this skill:\n\n{skill_md}");

        let python_code = self
            .llm_sub_call("anthropic/claude-sonnet-4-5", impl_system, &impl_user)
            .await?;

        // ── Step 3: Write files ───────────────────────────────────────────────
        let skills_dir = std::path::Path::new(&self.skills_dir);
        let scripts_dir = std::path::Path::new("scripts").join("skills");

        std::fs::create_dir_all(&scripts_dir)
            .context("failed to create scripts/skills directory")?;

        let md_path = skills_dir.join(format!("{name}.md"));
        std::fs::write(&md_path, &skill_md)
            .with_context(|| format!("failed to write {}", md_path.display()))?;

        let py_path = scripts_dir.join(format!("{name}.py"));
        std::fs::write(&py_path, &python_code)
            .with_context(|| format!("failed to write {}", py_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&py_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&py_path, perms)?;
        }

        tracing::info!(
            skill = %name,
            md = %md_path.display(),
            py = %py_path.display(),
            "create_skill: complete"
        );

        Ok(format!(
            "Skill '{name}' created successfully!\n\
             - Spec: {}\n\
             - Script: {}\n\
             The skill will be available on your next message.",
            md_path.display(),
            py_path.display()
        ))
    }

    async fn tool_execute_shell_command(&self, input: &serde_json::Value) -> Result<String> {
        let command = input["command"].as_str().unwrap_or("").trim();
        if command.is_empty() {
            return Ok("Error: 'command' field is required".to_string());
        }

        if is_dangerous_command(command) {
            tracing::warn!(command = %command, "tool_execute_shell_command: blocked dangerous command pattern");
            return Ok("Error: Command blocked — contains a potentially destructive pattern. If you intended this, rephrase your request.".to_string());
        }

        tracing::info!(command = %command, "tool_execute_shell_command: executing");

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::process::Command::new("bash")
                .arg("-c")
                .arg(command)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Shell command timed out after 30 seconds"))?
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
        // 0. Check for skill activation intent before anything else.
        if let Some((env_key, api_key_value)) = detect_skill_activation(user_message) {
            // Set in current process so subsequent tool calls in this process see it.
            std::env::set_var(&env_key, &api_key_value);
            // Persist to Keychain for future restarts.
            if let Err(e) = trusty_core::secrets::set(&env_key, &api_key_value) {
                tracing::warn!(env_key = %env_key, error = %e, "failed to persist skill key to Keychain");
            }
            // Find display name for confirmation message.
            let display_name = SKILL_CATALOG
                .iter()
                .find(|(k, _)| *k == env_key.as_str())
                .map(|(_, info)| info.name)
                .unwrap_or(&env_key);
            let reply = format!(
                "Done! The **{}** skill is now active. I've saved your key so it persists across restarts. Try asking me something that uses it.",
                display_name
            );
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
            session.messages.push(ChatMessage {
                id: Uuid::new_v4(),
                session_id: session.id,
                role: MessageRole::Assistant,
                content: reply.clone(),
                tool_name: None,
                tool_result: None,
                token_count: None,
                created_at: chrono::Utc::now(),
            });
            return Ok(StructuredResponse {
                reply,
                memories_to_save: vec![],
                referenced_entities: vec![],
                tool_calls: vec![],
            });
        }

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
        // Load current user preferences for system prompt injection.
        let current_prefs: Vec<(String, String)> = self
            .sqlite
            .as_deref()
            .and_then(|s| s.list_all_prefs().ok())
            .unwrap_or_default();
        let mut skills_content = crate::skills::load_skills(&self.skills_dir);
        for skill in &self.skills {
            if let Some(contribution) = skill.system_prompt_contribution() {
                if !skills_content.is_empty() {
                    skills_content.push_str("\n\n");
                }
                skills_content.push_str(&contribution);
            }
        }
        // Hot-reload script skills each turn so create_skill takes effect immediately.
        let script_skills =
            trusty_skill::load_script_skills(std::path::Path::new(&self.skills_dir));
        for skill in &script_skills {
            if let Some(contribution) = skill.system_prompt_contribution() {
                if !skills_content.is_empty() {
                    skills_content.push_str("\n\n");
                }
                skills_content.push_str(&contribution);
            }
        }
        let user_location = self
            .sqlite
            .as_deref()
            .and_then(|s| s.get_config("user_current_location").ok().flatten())
            .unwrap_or_default();
        let communication_style = self
            .sqlite
            .as_deref()
            .and_then(|s| {
                s.get_config("communication_style_work")
                    .ok()
                    .flatten()
                    .or_else(|| s.get_config("communication_style_personal").ok().flatten())
            })
            .unwrap_or_default();
        let system_content = system_prompt_inner(
            now,
            &context_section,
            &accounts_context,
            &current_prefs,
            &skills_content,
            &user_location,
            &communication_style,
            &self.instance_label,
        );

        // 3. Build the LLM message array from session history.
        let mut llm_messages: Vec<OrchatMessage> = vec![OrchatMessage {
            role: "system".to_string(),
            content: system_content,
        }];
        for msg in &session.messages {
            let role = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                // Anthropic has no "tool" role in the OpenAI-compat API without tool_call_id.
                // Treat persisted tool results as user messages so history alternates correctly.
                MessageRole::Tool => "user",
                MessageRole::System => "system",
            };
            llm_messages.push(OrchatMessage {
                role: role.to_string(),
                content: msg.content.clone(),
            });
        }

        // Merge consecutive same-role messages to satisfy Anthropic's strict alternation requirement.
        // Consecutive user or assistant messages get concatenated with "\n\n".
        let mut merged: Vec<OrchatMessage> = Vec::with_capacity(llm_messages.len());
        for msg in llm_messages {
            if let Some(last) = merged.last_mut() {
                if last.role == msg.role {
                    last.content = format!("{}\n\n{}", last.content, msg.content);
                    continue;
                }
            }
            merged.push(msg);
        }
        let mut llm_messages = merged;

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

            let mut attempts = 0u8;
            let http_response = loop {
                let resp = self
                    .http
                    .post(&url)
                    .bearer_auth(&self.api_key)
                    .json(&request_body)
                    .send()
                    .await
                    .context("failed to send request to OpenRouter")?;
                if resp.status().is_server_error() && attempts < 1 {
                    attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    continue;
                }
                break resp;
            };

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
                let raw = self
                    .execute_tool_by_name(&tc.name, &tc.arguments)
                    .await
                    .unwrap_or_else(|e| format!("Error: {e}"));
                tracing::info!(tool = %tc.name, "tool executed");
                let result = format_skill_suggestion(&raw);
                results_text.push_str(&format!("Tool `{}` returned:\n{}\n\n", tc.name, result));
            }

            // Append the assistant's tool-request turn and the results to the LLM context.
            llm_messages.push(OrchatMessage {
                role: "assistant".to_string(),
                content: raw_content.clone(),
            });
            llm_messages.push(OrchatMessage {
                role: "user".to_string(),
                content: format!(
                    "Tool results:\n\n{}\n\n\u{26a0}\u{fe0f} RESPOND WITH JSON ONLY \u{2014} no preamble, no explanation. Your response must be exactly:\n{{\"reply\":\"<your answer here>\",\"toolCalls\":[],\"memoriesToSave\":[],\"referencedEntities\":[]}}",
                    results_text
                ),
            });

            // NOTE: Intermediate tool call/result pairs are intentionally NOT persisted to
            // session.messages. Storing them caused two bugs:
            //   1. The "⚠️ RESPOND WITH JSON ONLY" injection leaked into persistent history,
            //      causing the LLM to emit raw JSON on the very next conversation turn.
            //   2. "[Tool calls: ...]" assistant messages in history confused the LLM into
            //      repeating tool calls or echoing internal markers as user-visible replies.
            // The tool call/result pairs live only in llm_messages (current-turn in-memory
            // context). Only the user message (step 1) and final reply (step 5) are persisted.
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

/// Patterns that are never allowed, regardless of context.
const BLOCKED_SHELL_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -r /",
    "rm -r ~",
    "> /dev/sda",
    "dd if=",
    "mkfs",
    ":(){ :|:& };:", // fork bomb
    "curl | bash",
    "curl | sh",
    "wget | bash",
    "wget | sh",
    "| bash",
    "| sh",
    "sudo ",
    "sudo\t",
    "pkill",
    "killall",
    "launchctl unload",
    "defaults delete",
    "chmod 777 /",
    "chown root",
];

fn is_dangerous_command(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    BLOCKED_SHELL_PATTERNS.iter().any(|pat| lower.contains(pat))
}

// ── Skill catalog ─────────────────────────────────────────────────────────────

struct SkillInfo {
    name: &'static str,
    scope_summary: &'static str,
    privacy_note: &'static str,
    get_key_url: &'static str,
    activate_name: &'static str,
}

const SKILL_CATALOG: &[(&str, SkillInfo)] = &[
    (
        "TAVILY_API_KEY",
        SkillInfo {
            name: "Web Search",
            scope_summary: "Searches the live web using Tavily and/or Brave. Queries and URLs leave your device.",
            privacy_note: "Search queries are sent to Tavily's servers. No personal data is included.",
            get_key_url: "https://app.tavily.com",
            activate_name: "web search",
        },
    ),
    (
        "BRAVE_SEARCH_API_KEY",
        SkillInfo {
            name: "Web Search",
            scope_summary: "Searches the live web using Tavily and/or Brave. Queries and URLs leave your device.",
            privacy_note: "Search queries are sent to Brave's servers. No personal data is included.",
            get_key_url: "https://api.search.brave.com/register",
            activate_name: "web search",
        },
    ),
    (
        "FIRECRAWL_API_KEY",
        SkillInfo {
            name: "Web Scrape",
            scope_summary: "Fetches full page content from URLs via Firecrawl. Only URLs leave your device.",
            privacy_note: "Only the URL is sent to Firecrawl; they don't retain content per their privacy policy.",
            get_key_url: "https://www.firecrawl.dev",
            activate_name: "web scrape",
        },
    ),
    (
        "SKYVERN_API_KEY",
        SkillInfo {
            name: "Browser Automation",
            scope_summary: "Controls a browser to interact with websites on your behalf via Skyvern.",
            privacy_note: "Skyvern runs in their cloud. Any credentials or data you provide in task instructions will be processed by their servers.",
            get_key_url: "https://app.skyvern.com",
            activate_name: "browser automation",
        },
    ),
    (
        "SERPAPI_API_KEY",
        SkillInfo {
            name: "Google Search",
            scope_summary: "Google-quality structured search via SerpApi. Search queries leave your device.",
            privacy_note: "Your queries are sent to SerpApi's servers, which proxy Google search.",
            get_key_url: "https://serpapi.com",
            activate_name: "google search",
        },
    ),
];

/// If `tool_output` is a "not configured" message, replace it with a
/// user-friendly suggestion that names the skill and explains how to activate it.
fn format_skill_suggestion(tool_output: &str) -> String {
    // Pattern: "<Name> not configured (<ENV_KEY> missing)"
    for (env_key, info) in SKILL_CATALOG {
        if tool_output.contains(env_key) && tool_output.contains("not configured") {
            return format!(
                "I can help with that, but it requires the **{}** skill which isn't set up yet.\n\n\
                **What it does:** {}\n\
                **Privacy:** {}\n\
                **To activate:** Get an API key at {}, then tell me: \
                \"activate {} with key [your-key]\"",
                info.name,
                info.scope_summary,
                info.privacy_note,
                info.get_key_url,
                info.activate_name,
            );
        }
    }
    tool_output.to_string()
}

/// Detect messages where the user is providing an API key for a skill.
/// Returns `(env_key, api_key_value)` when a match is found.
///
/// Recognised patterns (case-insensitive skill name):
///   "activate tavily with key tvly-abc"
///   "set up firecrawl, my key is fc-abc"
///   "my tavily key is tvly-abc"
///   "install serpapi key f9938..."
fn detect_skill_activation(msg: &str) -> Option<(String, String)> {
    let lower = msg.to_lowercase();

    // Map recognisable aliases to env keys.
    let aliases: &[(&str, &str)] = &[
        ("tavily", "TAVILY_API_KEY"),
        ("brave", "BRAVE_SEARCH_API_KEY"),
        ("web search", "TAVILY_API_KEY"),
        ("firecrawl", "FIRECRAWL_API_KEY"),
        ("web scrape", "FIRECRAWL_API_KEY"),
        ("skyvern", "SKYVERN_API_KEY"),
        ("browser automation", "SKYVERN_API_KEY"),
        ("serpapi", "SERPAPI_API_KEY"),
        ("google search", "SERPAPI_API_KEY"),
    ];

    for (alias, env_key) in aliases {
        if !lower.contains(alias) {
            continue;
        }
        // Look for a key value: a contiguous non-whitespace token that looks like
        // an API key (letters/digits/hyphens, at least 8 chars).
        let words: Vec<&str> = msg.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            let w = word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-');
            let is_key_word = w.len() >= 8
                && w.chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.');
            // Skip if the word is a known keyword (not a key value).
            let lower_w = w.to_lowercase();
            if is_key_word
                && ![
                    "activate", "setup", "install", "key", "with", "my", "is", alias,
                ]
                .contains(&lower_w.as_str())
                && !lower_w.starts_with(alias)   // skip tool names like "firecrawl_scrape"
                && i > 0
            {
                return Some((env_key.to_string(), w.to_string()));
            }
        }
    }
    None
}

#[cfg(test)]
fn system_prompt(
    now: chrono::DateTime<chrono::Utc>,
    context: &str,
    accounts_ctx: &str,
    current_prefs: &[(String, String)],
    skills_content: &str,
    user_location: &str,
    communication_style: &str,
) -> String {
    system_prompt_inner(
        now,
        context,
        accounts_ctx,
        current_prefs,
        skills_content,
        user_location,
        communication_style,
        "",
    )
}

#[allow(clippy::too_many_arguments)]
fn system_prompt_inner(
    now: chrono::DateTime<chrono::Utc>,
    context: &str,
    accounts_ctx: &str,
    current_prefs: &[(String, String)],
    skills_content: &str,
    user_location: &str,
    communication_style: &str,
    instance_label: &str,
) -> String {
    let user_email =
        std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
    let user_name = std::env::var("TRUSTY_USER_NAME").unwrap_or_else(|_| "Masa".to_string());
    let dev_badge = if !instance_label.is_empty() {
        format!(
            "**INSTANCE: {}** — You are running in a development/test instance. Data and conversations here are for testing only.\n\n",
            instance_label
        )
    } else {
        String::new()
    };
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
    let prefs_section = if current_prefs.is_empty() {
        String::new()
    } else {
        let mut s = "\n\n## Current Settings\n".to_string();
        for (key, value) in current_prefs {
            s.push_str(&format!("- {key}: {value}\n"));
        }
        s
    };
    let skills_section = if skills_content.is_empty() {
        String::new()
    } else {
        skills_content.to_string()
    };

    // Skill key presence — used to conditionally show tools in the system prompt
    let has_brave = trusty_core::secrets::get("BRAVE_SEARCH_API_KEY").is_some();
    let has_tavily = trusty_core::secrets::get("TAVILY_API_KEY").is_some();
    let has_firecrawl = trusty_core::secrets::get("FIRECRAWL_API_KEY").is_some();
    let has_skyvern = trusty_core::secrets::get("SKYVERN_API_KEY").is_some();
    let has_serpapi = trusty_core::secrets::get("SERPAPI_API_KEY").is_some();

    // ── "What I Can Do" capability bullets ──────────────────────────────────
    let cap_web_search = if has_brave {
        "- **Web Search**: I can search the web in real time via `web_search` (Brave \
Search). Use this for current events, news, prices, or anything that may have changed \
since my training.\n"
    } else {
        ""
    };
    let cap_tavily = if has_tavily {
        "- **Tavily Search**: Use `tavily_search` for research questions where you want \
a direct AI-synthesized answer with cited sources.\n"
    } else {
        ""
    };
    let cap_firecrawl = if has_firecrawl {
        "- **Web Scrape**: Use `firecrawl_scrape` when the user shares a URL and wants \
its full content extracted. Returns clean markdown.\n"
    } else {
        ""
    };
    let cap_skyvern = if has_skyvern {
        "- **Browser Automation**: Use `skyvern_task` when the user needs to interact \
with a website — fill forms, submit, click, or extract data from a navigated page. \
Tasks take 30–90 seconds.\n"
    } else {
        ""
    };
    let cap_serpapi = if has_serpapi {
        "- **Google Search**: Use `serpapi_search` for Google-quality structured search \
with knowledge graph, answer boxes, and organic results.\n"
    } else {
        ""
    };

    // ── "Available Tools" reference cards ───────────────────────────────────
    let tool_web_search = if has_brave {
        "- `web_search`: Search the web (Brave). Required: query. Optional: count (default 5, max 10).\n"
    } else {
        ""
    };
    let tool_tavily = if has_tavily {
        "- `tavily_search`: AI-optimised web search with synthesised answer + sources. Required: query.\n"
    } else {
        ""
    };
    let tool_firecrawl = if has_firecrawl {
        "- `firecrawl_scrape`: Extract a URL as clean markdown. Required: url. Max 4000 chars.\n"
    } else {
        ""
    };
    let tool_skyvern = if has_skyvern {
        "- `skyvern_task`: Browser automation. Required: url, goal. Optional: extract. Polls up to 60s.\n"
    } else {
        ""
    };
    let tool_serpapi = if has_serpapi {
        "- `serpapi_search`: Google search via SerpApi. Required: query. Optional: engine (google|bing|youtube|scholar, default google).\n"
    } else {
        ""
    };

    // ── Anti-hallucination routing hints ────────────────────────────────────
    let route_web: String = {
        let tools: Vec<&str> = [
            if has_brave {
                Some("`web_search`")
            } else {
                None
            },
            if has_tavily {
                Some("`tavily_search`")
            } else {
                None
            },
        ]
        .into_iter()
        .flatten()
        .collect();
        if tools.is_empty() {
            String::new()
        } else {
            format!(
                "- Current events, news, real-time info, prices → {}\n",
                tools.join(" or ")
            )
        }
    };
    let route_serpapi = if has_serpapi {
        "- Google-quality search with rich snippets or specific engine (bing/youtube/scholar) → `serpapi_search`\n"
    } else {
        ""
    };
    let route_firecrawl = if has_firecrawl {
        "- User pastes a URL and wants to read or summarise it → `firecrawl_scrape`\n"
    } else {
        ""
    };
    let route_skyvern = if has_skyvern {
        "- User needs to interact with a website (forms, clicks, navigation) → `skyvern_task`\n"
    } else {
        ""
    };

    let style_section = if communication_style.is_empty() {
        String::new()
    } else {
        let style: serde_json::Value =
            serde_json::from_str(communication_style).unwrap_or_default();
        let summary = style["summary"].as_str().unwrap_or("");
        let template = style["notification_template"].as_str().unwrap_or("");
        let formality = style["formality"].as_str().unwrap_or("");
        if summary.is_empty() {
            String::new()
        } else {
            format!(
                "\n- **Writing style**: {summary}\n\
                 - **Formality**: {formality}\n\
                 - **Notification template**: {template}"
            )
        }
    };
    format!(
        r#"{dev_badge}You are trusty-izzie, a personal AI assistant with deep knowledge of the user's professional relationships and work context. You run locally on the user's machine.

Today is {}. Current time: {}.

## About Your User
- **Name**: {user_name}
- **Email**: {user_email}
- **Timezone**: America/New_York (home base — may be travelling, see location below)
- You are their personal assistant. Address them by name when appropriate. When they ask who they are or about themselves, use this information.
- **Current location**: {user_location_line}
- **Location awareness**: When the user mentions being somewhere ("I'm in Berlin", "just landed in Tokyo", "heading to London"), treat it as their current location and save it as a memory with category "location". Surface this naturally when relevant — e.g. if they ask about weather, restaurants, local time, or train schedules.{style_section}
{context_section}{accounts_section}{prefs_section}{skills_section}

## My Deployment

I am trusty-izzie v{}, running as macOS launchd services:
- Daemon (com.trusty-izzie.daemon) — event processing, Gmail sync
- API (com.trusty-izzie.api) — REST API on port 3456
- Telegram (com.trusty-izzie.telegram) — Telegram bot on port 3457

I can check my own service status with `check_service_status`, report my version with `get_version`, file GitHub issues with `submit_github_issue`, and query my full operational state with `get_izzie_status`.

## What I Can Do
- **Skills discovery**: Use `search_skills` when unsure whether a capability exists — e.g. search "train" to find commute tools, "calendar" for scheduling tools.
- **Skill creation**: Use `create_skill` to build a brand-new skill from scratch. Opus designs the spec; Sonnet writes the Python implementation. The skill is live on your next message.
- **macOS Contacts**: I sync with your AddressBook via `sync_contacts`. I know your contact list.
- **Google Calendar**: I have access to your calendar via `get_calendar_events`. When asked about schedule, meetings, or upcoming events, I call this tool automatically. I can look ahead 1–30 days (default 7). Pass `account_email` to query a specific account (e.g. work calendar vs personal). I can also create new events via `create_calendar_event` and update existing events via `update_calendar_event`.
- **Google Tasks**: I fetch all task lists and tasks for an account in one call via `get_tasks_bulk`. Pass `account_email` to query a specific account. I also have `get_task_lists` and `get_tasks` for targeted operations. I can mark tasks complete via `complete_task`.
- **Weather**: I fetch real-time forecasts via `get_weather` (Open-Meteo, no API key) and active NWS severe weather alerts via `get_weather_alerts`. Default location is Hastings-on-Hudson, NY.
{cap_web_search}{cap_tavily}{cap_firecrawl}{cap_skyvern}{cap_serpapi}
- **Unified Search**: Use `search_all` to search across ALL sources simultaneously — memories, iMessage, Slack, calendar, tasks, and web — in a single call.
- **Email drafting**: I can draft emails and replies on your behalf using `send_email` / `reply_email`. All emails go into an approval queue — I show you the draft and wait for `approve <id>` before sending.
- **Task creation**: I can create Google Tasks directly via `create_task` (no approval needed).
- **Slack search**: I can search Slack channels and workspace messages via `search_slack`.

## Available Tools (complete list)
- `check_service_status` — report running status of all trusty-izzie launchd services
- `get_izzie_status` — full operational status: connected accounts, active skills, knowledge base counts, integration health
- `get_version` — return the current binary version
- `submit_github_issue` — file a GitHub issue via the `gh` CLI
- `schedule_event` — schedule a background task (email_sync, contacts_sync, memory_decay, reminder, agent_run, etc.)
- `cancel_event` — cancel a pending scheduled event by ID
- `list_events` — list scheduled or recent events, optionally filtered by status
- `run_agent` — enqueue a background research agent task
- `list_agents` — list available agent definitions
- `get_calendar_events` — fetch upcoming calendar events from ALL connected accounts. Optional: days (default 7, max 30). Optional: account_email — ONLY pass this when user explicitly asks for ONE specific account; omit for all general schedule queries to get work + personal combined.
- `create_calendar_event` — create a new Google Calendar event. Required: account_email, title, start_datetime (RFC3339), end_datetime (RFC3339). Optional: description, attendees (array of email strings).
- `update_calendar_event` — update an existing Google Calendar event. Use `get_calendar_events` first to find the event_id. Required: calendar_id, event_id, account_email. Optional: summary, start_time (RFC3339), end_time (RFC3339), description, location.
- `get_tasks_bulk` — fetches ALL task lists and ALL tasks for one account in a single call. Use this instead of get_task_lists + get_tasks. Required param: account_email.
- `get_task_lists` — list the user's Google Task lists (optional: account_email to query a specific account)
- `get_tasks` — fetch tasks from a list (optional: account_email, list_id, max_results, show_completed; default: incomplete tasks from primary list)
- `complete_task(account_email, task_list_id, task_id)` — Mark a Google Task as complete. Use get_tasks_bulk first to find the task_list_id and task_id.
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
- `get_train_schedule`: Fetch real-time Metro North departures between two stations. Required: from_station (e.g. "Hastings-on-Hudson", "Grand Central"), to_station. Optional: count (default 5, max 20). Returns upcoming train times with delays.
- `get_train_alerts`: Fetch active Metro North service alerts and delays. Optional: line (e.g. "Hudson", "New Haven", "Harlem"). Returns current disruptions.
- `get_weather`: Get weather forecast for a location. Optional: location (default: Hastings-on-Hudson), days (1-7, default 3). Returns daily summary + next 6 hours detail.
- `get_weather_alerts`: Get active NWS severe weather alerts for a location (US only). Optional: location (default: Hastings-on-Hudson).
- `search_skills`: Discover available skills by keyword. Required: query (string). Returns matching skill names, descriptions, and tool names. Use when unsure if a capability exists.
{tool_web_search}- `fetch_page`: Fetch and read the text content of a URL. Required: url (string). Optional: max_chars (default 3000, max 8000). Use after web_search to get full article/review content.
{tool_tavily}{tool_firecrawl}{tool_skyvern}{tool_serpapi}
- `get_izzie_status`: Check Izzie's operational status — connected Gmail accounts, active skills, integration health, knowledge base size. Call when user asks "are you connected?", "what accounts do you have?", "what can you do?", "what skills are active?", or similar.
- `create_skill`: Design and build a new skill. Uses Opus to architect the skill spec and Sonnet to write the Python implementation. Required: name (kebab-case, e.g. "hacker-news"), description (plain English — what it fetches, which APIs, etc.). The skill is available on the next turn.

## Acting on Your Behalf (Search + Act)

I can search across all your data and act on your behalf — with your approval for sensitive actions.

### Search
- `search_all`: Unified search across ALL sources at once — memories, iMessage, Slack, calendar, tasks, and web. Required: query. Optional: sources (array of "memories","entities","imessage","slack","calendar","tasks","web" — defaults to all). Returns labeled results from each source.
- `search_slack`: Search Slack messages. Required: query. Optional: channel_id (specific channel via bot token), limit (default 10). With user token, also does workspace-wide search.

### Email Actions (require approval)
- `send_email`: Draft a new email for your approval. Required: account_email, to (array of addresses), subject, body. Optional: cc. Returns an approval prompt — confirm with `approve <id>`.
- `reply_email`: Draft a reply for your approval. Required: account_email, thread_id, to, subject, body. Returns an approval prompt — confirm with `approve <id>`.

### Task Actions
- `create_task`: Create a new Google Task immediately (no approval required). Required: account_email, title. Optional: task_list_id (default: primary list), notes, due (RFC3339 date string).

### Approval Queue
- `list_pending_actions`: Show all actions waiting for your approval with their short IDs.
- `approve_action`: Execute an approved action. Required: id (full UUID or short prefix, e.g. "a3f2"). Runs the action and reports success/failure.
- `reject_action`: Discard a pending action. Required: id (full UUID or short prefix).

**Approval workflow**: When I draft an email or other sensitive action, I return a prompt with a short ID. You confirm with `approve <id>` or `reject <id>`. You can also say "yes", "send it", or "go ahead" — I'll look up and approve the most recent pending action.

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

Note: Destructive commands (rm -rf, sudo, pkill, pipe-to-shell, dd, mkfs) are automatically blocked.
Use execute_shell_command for read-only operations: ls, cat, grep, find, ps, df, etc.

## Tool Calling Protocol

To invoke a tool, set `toolCalls` to a non-empty array. Leave `reply` empty when requesting tools — the user won't see it until you give your final response:

{{"reply":"","toolCalls":[{{"name":"get_calendar_events","arguments":{{"days":7}}}}],"memoriesToSave":[],"referencedEntities":[]}}

After tool results are injected into the conversation, give your final answer with `toolCalls` empty:

{{"reply":"Here is your schedule for the next week...","toolCalls":[],"memoriesToSave":[],"referencedEntities":[]}}

## Anti-Hallucination Rules

NEVER fabricate factual information. For these topics you MUST call the appropriate tool — never answer from memory or training data:
- Calendar / schedule / meetings → `get_calendar_events`
- Scheduled tasks / reminders / events → `list_events`
- Tasks / to-dos → `get_tasks_bulk` (preferred) or `get_task_lists` + `get_tasks`
- Google accounts → `list_accounts`
- Service health / running processes → `check_service_status`
- Connection status, linked accounts, active skills, integration health → `get_izzie_status`
- Any file system, shell, or system state query → `execute_shell_command`
- User preferences → `get_preferences`
- Contact info (phone, email, address) → `search_contacts` ALWAYS before answering
- iMessage history → `search_imessages` ALWAYS; never fabricate message content
- WhatsApp history → `search_whatsapp` ALWAYS; never fabricate message content
- Train schedules / Metro North / commute timing → `get_train_schedule` ALWAYS; never guess departure times
- Train delays / service alerts → `get_train_alerts` ALWAYS
- Weather / forecast / temperature / rain / snow → `get_weather` ALWAYS; never guess weather from training data
- Severe weather / storm warnings / alerts → `get_weather_alerts`
{route_web}{route_serpapi}{route_firecrawl}{route_skyvern}

If a tool returns no data (e.g. no calendar events), say so honestly. Never invent meetings, contacts, emails, or any factual data.

## Identity & Account Inference

**MANDATORY CALENDAR RULE**: When the user asks about their schedule, agenda, meetings, or calendar for any day/period — you MUST call `get_calendar_events` WITHOUT `account_email`. This single call automatically queries ALL connected accounts (personal: bob@matsuoka.com, work: robert.matsuoka@duettoresearch.com, and bobmatnyc@gmail.com) and returns combined results labeled by account. Never call with a specific `account_email` for general schedule queries — only use `account_email` when the user explicitly asks about ONE specific account (e.g. "check my work calendar only").

**MANDATORY TASKS RULE**: When the user asks about tasks or to-dos, you MUST call `get_tasks_bulk` once for EACH account that has "tasks" in its capabilities list. This single call returns all lists and all tasks for that account. Combine results from all accounts before replying. Never query only one account.

**MANDATORY SCHEDULING RULE**: When someone says "let me know what works", "my [time] is open", or asks to schedule a meeting:
1. Call `search_contacts` if you have a phone number or name to identify the person
2. Call `get_calendar_events` for the relevant days to find free slots
3. Suggest 2–3 specific times (with day, date, and time)
4. Ask the user to confirm which slot works, then call `create_calendar_event` with their choice

When the user asks about calendar, email, or tasks, infer which account to use from context:

**Work account signals:**
- Mentions of colleagues, boss, manager, client, team, company, office
- Keywords: meeting, standup, sprint, deadline, invoice, project, task, PR, ticket
- Professional context: "my 3pm", "the team call", "Q1 review"
- Work hours context (when time is mentioned: 9am–6pm weekday)

**Personal account signals:**
- Mentions of family: wife, husband, partner, kids, parents, friend
- Keywords: vacation, weekend, dentist, gym, dinner, birthday, holiday
- Personal context: "our trip", "family dinner", "personal project"

**When ambiguous or spanning both:**
- Check both accounts and clearly label results: "On your work calendar... / On your personal calendar..."
- Prefer work account during work hours (Mon–Fri 9am–6pm) if truly ambiguous

**Always attribute results:**
- "Your work calendar (email@company.com) shows..."
- "On your personal calendar (email@gmail.com)..."
- Never silently pick one without indicating which account you used

## Preference Capture Rules

When the user expresses a preference about your behavior, IMMEDIATELY call `set_preference` to persist it. Do not just acknowledge — always save it.

Examples of preference statements:
- "be more concise" → set_preference(key="response_style", value="concise")
- "stop morning messages" → set_preference(key="morning_briefing_enabled", value="false")
- "I want evening summaries" → set_preference(key="evening_briefing_enabled", value="true")
- "less personal, more task-focused" → set_preference(key="tone", value="professional")
- "remind me about X weekly" → set_preference(key="weekly_digest_enabled", value="true")

Valid preference keys and defaults:
| Key | Default | Description |
|-----|---------|-------------|
| morning_briefing_enabled | true | 8am daily briefing |
| evening_briefing_enabled | true | 6pm daily briefing |
| weekly_digest_enabled | true | Monday 9am digest |
| response_style | balanced | "concise" or "balanced" or "detailed" |
| tone | friendly | "friendly" or "professional" |
| interrupt_notifications_enabled | true | iMessage/WhatsApp push alerts |
| vip_email_alerts_enabled | true | VIP contact email alerts |

After saving, confirm: "Got it — I've saved that preference."

## Action Verification (CRITICAL — NEVER VIOLATE)

After calling any state-mutating tool (create_event, create_task, send_email, reply_email, update_event, delete_event):
- ONLY report success if the tool result contains "confirmed id:" or "sent successfully" or "created successfully" or "Queued for your approval"
- If the tool result contains "UNCONFIRMED" or "Failed" or "error" — tell the user exactly: "I attempted [action] but could not confirm it completed. [reason]. Please verify manually."
- NEVER say "I've created", "I've sent", "I've scheduled", "Done!" unless the tool confirmed it with an ID or approval queue entry
- The action is not done until the tool returns a confirmation ID or approval queue entry

## ABSOLUTE RULE — NEVER CLAIM SUCCESS WITHOUT A TOOL RESULT

You MUST NOT write phrases like "I've successfully updated", "Done!", "I've sent", "I've created", "I've deleted", or any claim of completed action UNLESS you have just received a tool result in this same response that confirms it with a resource ID.

If you don't have the right tool for what the user asked:
- Say so IMMEDIATELY in your first sentence
- Do NOT describe what you "would have done"
- Do NOT describe a fake success and then walk it back
- Offer concrete alternatives (use a different tool, ask the user to do it manually, etc.)

If a tool returns an error or UNCONFIRMED:
- Tell the user the action did NOT complete
- State what went wrong
- Offer to retry or suggest manual steps

## ABSOLUTE RULE — ONLY REPORT WHAT TOOLS ACTUALLY RETURNED

When you call get_calendar_events, get_tasks, list_emails, search_all, or any other
data retrieval tool, you MUST report ONLY what the tool result contained.
- NEVER invent, guess, or "fill in" events, meetings, tasks, or emails that were not
  in the tool result.
- NEVER add agenda items, attendee names, or details beyond what the tool returned.
- If the tool returned no events, say "No events found" — do not substitute events
  from memory or training data.
- If a meeting title is "AGM Prep", report it as "AGM Prep". Do not rename it or
  add context like "(Board meeting preparation)" unless the description field
  explicitly contained that text.

## CRITICAL OUTPUT FORMAT

Your ENTIRE response must be a single raw JSON object. Output ONLY the JSON — no prose before it, no explanation after it, no markdown code fences around it. Start your response with {{ and end with }}.

Required format (output this and nothing else):
{{"reply":"your response to the user (markdown allowed)","toolCalls":[],"memoriesToSave":[],"referencedEntities":[]}}

## Memory Schema (CRITICAL — follow exactly)

When saving memories, use structured JSON content for these types:

**Person profile** (use when user shares info about a person):
{{"type":"person_profile","name":"...","email":"...","relationship":"...","interests":[...],"notes":"..."}}

**Recurring pattern** (use for habits, preferences, routines):
{{"type":"recurring_pattern","description":"...","frequency":"...","last":"...","next":"..."}}

**Reminder** (use for time-based follow-ups):
{{"type":"reminder","date":"YYYY-MM-DD","subject":"...","context":"..."}}

**User preference** (use for how user wants Izzie to behave):
{{"type":"preference","key":"...","value":"...","context":"..."}}

Plain text is acceptable for one-off facts that do not fit the above categories.

IMPORTANT: When the user says "remember X about Y" or "save info on Y", save it as person_profile JSON. Include ALL details provided — do not summarise to basics. A comprehensive profile in one memory is better than many sparse ones.

**Category field must match**: person_profile/recurring_pattern → "general"; reminder → "reminder"; preference → "user_preference"

Example with a structured memory (note: `category` MUST be a plain string, NOT a JSON object):
{{"reply":"Got it!","toolCalls":[],"memoriesToSave":[{{"content":"{{\"type\":\"person_profile\",\"name\":\"Alice\",\"email\":\"alice@example.com\",\"relationship\":\"colleague\"}}","category":"general","importance":0.8}}],"referencedEntities":[]}}

IMPORTANT: The "reply" field must ALWAYS be non-empty in your final response (when `toolCalls` is empty). Even for declarative statements, acknowledge receipt — e.g. "Got it, noted!" Never leave "reply" empty in a final response.
IMPORTANT: `category` must be a plain string (e.g. "reminder", "user_preference"), NOT a JSON object like {{"Reminder": null}}.

Be helpful, concise, and honest. Only include items in memoriesToSave if you learned something genuinely new and useful. Be selective — 0-1 memories per turn is typical."#,
        now.with_timezone(&chrono::Local).format("%A, %B %d, %Y"),
        {
            let local = now.with_timezone(&chrono::Local);
            format!("{} ({})", local.format("%H:%M"), local.format("%Z %:z"))
        },
        env!("CARGO_PKG_VERSION"),
        user_location_line = if user_location.is_empty() {
            "Hastings-on-Hudson, NY (home — no recent location update)".to_string()
        } else {
            user_location.to_string()
        },
        cap_web_search = cap_web_search,
        cap_tavily = cap_tavily,
        cap_firecrawl = cap_firecrawl,
        cap_skyvern = cap_skyvern,
        cap_serpapi = cap_serpapi,
        tool_web_search = tool_web_search,
        tool_tavily = tool_tavily,
        tool_firecrawl = tool_firecrawl,
        tool_skyvern = tool_skyvern,
        tool_serpapi = tool_serpapi,
        route_web = route_web,
        route_serpapi = route_serpapi,
        route_firecrawl = route_firecrawl,
        route_skyvern = route_skyvern,
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
    let mut first_json_err: Option<serde_json::Error> = None;
    let mut search = raw;
    while let Some(start) = search.find('{') {
        let candidate = &search[start..];
        match serde_json::from_str::<StructuredResponse>(candidate) {
            Ok(s) => return s,
            Err(e) => {
                if first_json_err.is_none() {
                    first_json_err = Some(e);
                }
            }
        }
        search = &search[start + 1..];
    }

    // 4. Fallback: treat the whole raw string as a plain-text reply.
    if raw.trim_start().starts_with('{') {
        tracing::warn!(
            "parse_response: JSON found but failed to deserialize: {}. First 200 chars: {:?}",
            first_json_err.map(|e| e.to_string()).unwrap_or_default(),
            &raw.chars().take(200).collect::<String>()
        );
    } else {
        tracing::debug!(
            "parse_response: plain text response (no JSON wrapper). First 200 chars: {:?}",
            &raw.chars().take(200).collect::<String>()
        );
    }
    StructuredResponse {
        reply: raw.to_string(),
        memories_to_save: vec![],
        referenced_entities: vec![],
        tool_calls: vec![],
    }
}

/// Remove any trailing ```json ... ``` block that the model sometimes appends
/// inside the reply field as a "structured output" summary.
/// Also guards against the reply field itself being a raw StructuredResponse JSON blob.
fn clean_reply(reply: &str) -> String {
    let trimmed = reply.trim();
    // Safety net: if the whole reply is a JSON object with a "reply" key, extract it.
    // This fires when parse_response falls back to raw text because deserialization
    // failed (e.g. MemoryCategory sent as a map) and the entire JSON blob lands here.
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(inner_reply) = v.get("reply").and_then(|r| r.as_str()) {
                if !inner_reply.is_empty() {
                    return clean_reply(inner_reply);
                }
            }
        }
    }
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

// ═══════════════════════════════════════════════════════════════════════════
// Email: send + reply
// ═══════════════════════════════════════════════════════════════════════════

impl ChatEngine {
    /// Queue an email to send for user approval, or send immediately if auto-approved.
    ///
    /// Parameters: account_email?, to, subject, body, cc?, bcc?, reply_to_thread_id?
    async fn tool_send_email(&self, input: &serde_json::Value) -> Result<String> {
        let primary =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = input["account_email"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&primary);

        let to = match input["to"].as_str().filter(|s| !s.is_empty()) {
            Some(t) => t,
            None => return Ok("Missing required parameter: to".into()),
        };
        let subject = input["subject"].as_str().unwrap_or("(no subject)");
        let body = match input["body"].as_str().filter(|s| !s.is_empty()) {
            Some(b) => b,
            None => return Ok("Missing required parameter: body".into()),
        };
        let cc = input["cc"].as_str().unwrap_or("");

        let description = format!("Send email to {to}: \"{subject}\"");
        let payload = serde_json::json!({
            "account_email": email,
            "to": to,
            "subject": subject,
            "body": body,
            "cc": cc,
        });

        let sqlite = self.sqlite_ref()?;
        let action_id = sqlite.queue_action("send_email", &description, &payload)?;

        Ok(format!(
            "📧 Queued for your approval (action ID: {action_id}).\n\
            **To**: {to}\n**Subject**: {subject}\n\n{body}\n\n\
            Reply `approve {action_id}` to send, or `reject {action_id}` to cancel."
        ))
    }

    /// Queue a reply to an existing Gmail thread.
    ///
    /// Parameters: account_email?, thread_id, message_id, to, subject, body
    async fn tool_reply_email(&self, input: &serde_json::Value) -> Result<String> {
        let primary =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = input["account_email"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&primary);

        let thread_id = match input["thread_id"].as_str().filter(|s| !s.is_empty()) {
            Some(t) => t,
            None => return Ok("Missing required parameter: thread_id".into()),
        };
        let to = match input["to"].as_str().filter(|s| !s.is_empty()) {
            Some(t) => t,
            None => return Ok("Missing required parameter: to".into()),
        };
        let subject = input["subject"].as_str().unwrap_or("Re: (no subject)");
        let body = match input["body"].as_str().filter(|s| !s.is_empty()) {
            Some(b) => b,
            None => return Ok("Missing required parameter: body".into()),
        };
        let in_reply_to = input["message_id"].as_str().unwrap_or("");

        let description = format!("Reply to thread {thread_id} → {to}: \"{subject}\"");
        let payload = serde_json::json!({
            "account_email": email,
            "thread_id": thread_id,
            "message_id": in_reply_to,
            "to": to,
            "subject": subject,
            "body": body,
        });

        let sqlite = self.sqlite_ref()?;
        let action_id = sqlite.queue_action("reply_email", &description, &payload)?;

        Ok(format!(
            "📧 Reply queued for your approval (action ID: {action_id}).\n\
            **To**: {to}\n**Subject**: {subject}\n\n{body}\n\n\
            Reply `approve {action_id}` to send, or `reject {action_id}` to cancel."
        ))
    }

    /// Actually send an email via Gmail API (called after user approves).
    pub async fn execute_send_email(&self, payload: &serde_json::Value) -> Result<String> {
        let primary =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = payload["account_email"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&primary);

        let access_token = self.get_valid_token(email).await?;

        let to = payload["to"].as_str().unwrap_or("");
        let subject = payload["subject"].as_str().unwrap_or("(no subject)");
        let body = payload["body"].as_str().unwrap_or("");
        let cc = payload["cc"].as_str().unwrap_or("");
        let thread_id = payload["thread_id"].as_str();
        let in_reply_to = payload["message_id"].as_str();

        // Build RFC 2822 message
        let mut raw = format!(
            "From: {email}\r\nTo: {to}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n"
        );
        if !cc.is_empty() {
            raw.push_str(&format!("Cc: {cc}\r\n"));
        }
        if let Some(msg_id) = in_reply_to {
            if !msg_id.is_empty() {
                raw.push_str(&format!(
                    "In-Reply-To: {msg_id}\r\nReferences: {msg_id}\r\n"
                ));
            }
        }
        raw.push_str(&format!("\r\n{body}"));

        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let encoded = URL_SAFE_NO_PAD.encode(raw.as_bytes());

        let mut req_body = serde_json::json!({ "raw": encoded });
        if let Some(tid) = thread_id {
            if !tid.is_empty() {
                req_body["threadId"] = serde_json::Value::String(tid.to_string());
            }
        }

        let resp = self
            .http
            .post("https://gmail.googleapis.com/gmail/v1/users/me/messages/send")
            .bearer_auth(&access_token)
            .json(&req_body)
            .send()
            .await?;

        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg_id = body["id"].as_str().unwrap_or("unknown");
            Ok(format!("Email sent (message ID: {msg_id})"))
        } else {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            Err(anyhow::anyhow!("Gmail send failed: {msg}"))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tasks: create
// ═══════════════════════════════════════════════════════════════════════════

impl ChatEngine {
    /// Create a new task in a Google Task list.
    ///
    /// Parameters: account_email?, task_list_id, title, notes?, due?
    async fn tool_create_task(&self, input: &serde_json::Value) -> Result<String> {
        let primary =
            std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
        let email = input["account_email"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&primary);
        let access_token = match self.get_valid_token(email).await {
            Ok(t) => t,
            Err(e) => return Ok(format!("Cannot access Tasks for {email}: {e}")),
        };

        let task_list_id = match input["task_list_id"].as_str().filter(|s| !s.is_empty()) {
            Some(id) => id,
            None => {
                return Ok(
                    "Missing required parameter: task_list_id (use get_task_lists to find it)"
                        .into(),
                )
            }
        };
        let title = match input["title"].as_str().filter(|s| !s.is_empty()) {
            Some(t) => t,
            None => return Ok("Missing required parameter: title".into()),
        };

        let mut body = serde_json::json!({ "title": title });
        if let Some(notes) = input["notes"].as_str().filter(|s| !s.is_empty()) {
            body["notes"] = serde_json::Value::String(notes.to_string());
        }
        if let Some(due) = input["due"].as_str().filter(|s| !s.is_empty()) {
            // due must be RFC 3339 format; append T00:00:00.000Z if only date given
            let due_formatted = if due.contains('T') {
                due.to_string()
            } else {
                format!("{due}T00:00:00.000Z")
            };
            body["due"] = serde_json::Value::String(due_formatted);
        }

        let url = format!(
            "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks",
            task_list_id
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&access_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let task_id = match body["id"].as_str() {
                Some(id) if !id.is_empty() => id,
                _ => {
                    return Ok(format!(
                        "Action UNCONFIRMED: task API returned no task ID for \"{title}\". \
                         The task may NOT have been created. Raw response: {}",
                        serde_json::to_string(&body)
                            .unwrap_or_default()
                            .chars()
                            .take(200)
                            .collect::<String>()
                    ))
                }
            };
            Ok(format!(
                "Task created (confirmed id: {task_id}): \"{title}\""
            ))
        } else {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            Ok(format!("Failed to create task (HTTP {status}): {msg}"))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Slack search
// ═══════════════════════════════════════════════════════════════════════════

impl ChatEngine {
    /// Search Slack channel/DM history by keyword.
    ///
    /// Parameters: query, channel_id?, limit?
    async fn tool_search_slack(&self, input: &serde_json::Value) -> Result<String> {
        let token = match trusty_core::secrets::get("SLACK_BOT_TOKEN") {
            Some(t) => t,
            None => return Ok("SLACK_BOT_TOKEN not configured.".into()),
        };
        let query = match input["query"].as_str().filter(|s| !s.is_empty()) {
            Some(q) => q,
            None => return Ok("Missing required parameter: query".into()),
        };
        let channel = input["channel_id"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(20).min(100);

        // Use conversations.history for a specific channel, or search.messages for workspace-wide
        if !channel.is_empty() {
            // Channel-specific: fetch recent messages and filter by query
            let resp: serde_json::Value = self
                .http
                .get("https://slack.com/api/conversations.history")
                .bearer_auth(&token)
                .query(&[("channel", channel), ("limit", &limit.to_string())])
                .send()
                .await?
                .json()
                .await?;

            if !resp["ok"].as_bool().unwrap_or(false) {
                let err = resp["error"].as_str().unwrap_or("unknown");
                return Ok(format!(
                    "Slack API error: {err}. Check channels:history scope."
                ));
            }

            let messages = resp["messages"].as_array().cloned().unwrap_or_default();
            let query_lower = query.to_lowercase();
            let matches: Vec<String> = messages
                .iter()
                .filter(|m| {
                    m["text"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query_lower)
                })
                .take(10)
                .map(|m| {
                    let ts = m["ts"].as_str().unwrap_or("");
                    let user = m["user"].as_str().unwrap_or("bot");
                    let text = m["text"].as_str().unwrap_or("");
                    let preview: String = text.chars().take(200).collect();
                    format!("[{user} @ {ts}]: {preview}")
                })
                .collect();

            if matches.is_empty() {
                Ok(format!("No messages in <#{channel}> matching \"{query}\"."))
            } else {
                Ok(format!(
                    "Found {} message(s) in <#{channel}> matching \"{query}\":\n{}",
                    matches.len(),
                    matches.join("\n")
                ))
            }
        } else {
            // Workspace-wide: use search.messages (requires search:read scope on user token)
            let user_token = trusty_core::secrets::get("SLACK_USER_TOKEN").unwrap_or(token);
            let resp: serde_json::Value = self
                .http
                .get("https://slack.com/api/search.messages")
                .bearer_auth(&user_token)
                .query(&[("query", query), ("count", &limit.to_string())])
                .send()
                .await?
                .json()
                .await?;

            if !resp["ok"].as_bool().unwrap_or(false) {
                let err = resp["error"].as_str().unwrap_or("unknown");
                return Ok(format!(
                    "Slack search error: {err}. Workspace-wide search requires search:read scope on user token."
                ));
            }

            let matches = resp["messages"]["matches"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if matches.is_empty() {
                return Ok(format!("No Slack messages found matching \"{query}\"."));
            }

            let results: Vec<String> = matches
                .iter()
                .take(10)
                .map(|m| {
                    let channel_name = m["channel"]["name"].as_str().unwrap_or("unknown");
                    let user = m["username"].as_str().unwrap_or("unknown");
                    let text = m["text"].as_str().unwrap_or("");
                    let preview: String = text.chars().take(200).collect();
                    format!("#{channel_name} [{user}]: {preview}")
                })
                .collect();

            Ok(format!(
                "Found {} Slack message(s) for \"{query}\":\n{}",
                results.len(),
                results.join("\n")
            ))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Unified search_all
// ═══════════════════════════════════════════════════════════════════════════

impl ChatEngine {
    /// Fan-out search across all personal data + web, RRF-merge results.
    ///
    /// Parameters: query, sources? (array of: "memories","entities","imessage","slack","calendar","tasks","web")
    async fn tool_search_all(&self, input: &serde_json::Value) -> Result<String> {
        let query = match input["query"].as_str().filter(|s| !s.is_empty()) {
            Some(q) => q,
            None => return Ok("Missing required parameter: query".into()),
        };

        // Determine which sources to query
        let all_sources = [
            "memories", "entities", "imessage", "slack", "calendar", "tasks", "web",
        ];
        let requested: Vec<&str> = if let Some(arr) = input["sources"].as_array() {
            arr.iter().filter_map(|v| v.as_str()).collect()
        } else {
            all_sources.to_vec()
        };

        let mut sections: Vec<String> = Vec::new();

        // Run all sources concurrently — call concrete methods directly to avoid async recursion
        let web_query = serde_json::json!({"query": query});
        let imsg_query = serde_json::json!({"query": query, "limit": 5});
        let slack_query = serde_json::json!({"query": query, "limit": 10});

        let (web_r, imsg_r) = tokio::join!(
            async {
                if requested.contains(&"web") {
                    self.tool_web_search(&web_query).await.ok()
                } else {
                    None
                }
            },
            async {
                if requested.contains(&"imessage") {
                    self.tool_search_imessages(&imsg_query).ok()
                } else {
                    None
                }
            },
        );

        let (slack_r, tasks_r) = tokio::join!(
            async {
                if requested.contains(&"slack") {
                    self.tool_search_slack(&slack_query).await.ok()
                } else {
                    None
                }
            },
            async {
                if requested.contains(&"tasks") {
                    let primary = std::env::var("TRUSTY_PRIMARY_EMAIL")
                        .unwrap_or_else(|_| PRIMARY_EMAIL.to_string());
                    self.tool_get_tasks_bulk(&serde_json::json!({"account_email": primary}))
                        .await
                        .ok()
                } else {
                    None
                }
            },
        );

        // Placeholders for memory/entity search (not yet implemented at store level)
        let mem_r: Option<String> = None;
        let entity_r: Option<String> = None;

        // Collect non-empty results with source labels
        if let Some(r) = mem_r {
            if !r.contains("No memories") && !r.is_empty() {
                sections.push(format!("**Memories:**\n{r}"));
            }
        }
        if let Some(r) = entity_r {
            if !r.contains("No entities") && !r.is_empty() {
                sections.push(format!("**Entities:**\n{r}"));
            }
        }
        if let Some(r) = imsg_r {
            if !r.contains("No messages") && !r.is_empty() {
                sections.push(format!("**iMessage:**\n{r}"));
            }
        }
        if let Some(r) = slack_r {
            if !r.contains("No Slack") && !r.is_empty() {
                sections.push(format!("**Slack:**\n{r}"));
            }
        }
        if let Some(r) = tasks_r {
            if !r.is_empty() {
                sections.push(format!("**Tasks:**\n{r}"));
            }
        }
        if let Some(r) = web_r {
            if !r.is_empty() {
                sections.push(format!("**Web:**\n{r}"));
            }
        }

        if sections.is_empty() {
            Ok(format!(
                "No results found across any source for \"{query}\"."
            ))
        } else {
            Ok(format!(
                "Search results for \"{query}\" ({} source{}):\n\n{}",
                sections.len(),
                if sections.len() == 1 { "" } else { "s" },
                sections.join("\n\n---\n\n")
            ))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Pending actions approval queue
// ═══════════════════════════════════════════════════════════════════════════

impl ChatEngine {
    fn tool_list_pending_actions(&self) -> Result<String> {
        let sqlite = self.sqlite_ref()?;
        let actions = sqlite.list_pending_actions()?;
        if actions.is_empty() {
            return Ok("No pending actions.".into());
        }
        let lines: Vec<String> = actions
            .iter()
            .map(|a| {
                format!(
                    "• **[{}]** `{}` — {}\n  _(queued {})_",
                    a.id.split('-').next().unwrap_or(&a.id),
                    a.action_type,
                    a.description,
                    chrono::DateTime::from_timestamp(a.proposed_at, 0)
                        .map(|dt: chrono::DateTime<chrono::Utc>| dt
                            .format("%Y-%m-%d %H:%M")
                            .to_string())
                        .unwrap_or_else(|| a.proposed_at.to_string())
                )
            })
            .collect();
        Ok(format!(
            "**{} pending action{}:**\n{}\n\nReply `approve <id>` or `reject <id>`.",
            actions.len(),
            if actions.len() == 1 { "" } else { "s" },
            lines.join("\n")
        ))
    }

    async fn tool_approve_action(&self, input: &serde_json::Value) -> Result<String> {
        let id = match input["id"].as_str().filter(|s| !s.is_empty()) {
            Some(i) => i,
            None => return Ok("Missing required parameter: id".into()),
        };

        let sqlite = self.sqlite_ref()?;

        // Support short IDs (first segment of UUID)
        let full_id = if id.len() < 36 {
            let actions = sqlite.list_pending_actions()?;
            actions
                .iter()
                .find(|a| a.id.starts_with(id))
                .map(|a| a.id.clone())
                .unwrap_or_else(|| id.to_string())
        } else {
            id.to_string()
        };

        let action = match sqlite.approve_action(&full_id)? {
            Some(a) => a,
            None => return Ok(format!("Action {id} not found or already resolved.")),
        };

        // Execute the action
        let payload: serde_json::Value = serde_json::from_str(&action.payload).unwrap_or_default();

        let result = match action.action_type.as_str() {
            "send_email" | "reply_email" => self.execute_send_email(&payload).await,
            other => Err(anyhow::anyhow!("Unknown action type: {other}")),
        };

        match result {
            Ok(msg) => {
                sqlite.mark_action_executed(&full_id, &msg)?;
                Ok(format!("✅ {msg}"))
            }
            Err(e) => {
                sqlite.mark_action_failed(&full_id, &e.to_string())?;
                Ok(format!("❌ Action failed: {e}"))
            }
        }
    }

    fn tool_reject_action(&self, input: &serde_json::Value) -> Result<String> {
        let id = match input["id"].as_str().filter(|s| !s.is_empty()) {
            Some(i) => i,
            None => return Ok("Missing required parameter: id".into()),
        };

        let sqlite = self.sqlite_ref()?;

        let full_id = if id.len() < 36 {
            let actions = sqlite.list_pending_actions()?;
            actions
                .iter()
                .find(|a| a.id.starts_with(id))
                .map(|a| a.id.clone())
                .unwrap_or_else(|| id.to_string())
        } else {
            id.to_string()
        };

        sqlite.reject_action(&full_id)?;
        Ok(format!("❌ Action {id} rejected and discarded."))
    }
}

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
        let prompt = system_prompt(now, "", "", &[], "", "", "");
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
            &[],
            "",
            "",
            "",
        );
        assert!(prompt.contains("## Relevant People & Projects"));
    }

    #[test]
    fn test_system_prompt_no_context_section_when_empty() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(now, "", "", &[], "", "", "");
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
