//! The core chat completion engine.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trusty_models::chat::{ChatMessage, ChatSession, MessageRole, StructuredResponse};
use trusty_models::{EventPayload, EventType};
use trusty_store::SqliteStore;

use crate::context::ContextAssembler;
use crate::tools::ToolName;

/// Drives the conversation loop: context assembly → LLM call → tool dispatch → save.
pub struct ChatEngine {
    http: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
    /// Reserved for the tool call loop (phase 2 — not yet implemented).
    #[allow(dead_code)]
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
            ToolName::ExecuteShellCommand => self.tool_execute_shell_command(input),
            _ => Ok("Tool not yet implemented.".to_string()),
        }
    }

    fn sqlite_ref(&self) -> Result<&SqliteStore> {
        self.sqlite
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Event queue unavailable: no SQLite store attached"))
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
    /// The caller is responsible for loading and saving the session.
    pub async fn chat(
        &self,
        session: &mut ChatSession,
        user_message: &str,
    ) -> Result<StructuredResponse> {
        // 1. Append user message
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

        // 2. Assemble RAG context from LanceDB entities and memories
        let ctx = self.context_assembler.assemble(user_message, "").await?;
        let context_section = self.context_assembler.render_context(&ctx);

        // 3. Build messages for OpenRouter
        let now = chrono::Utc::now();
        let system_content = system_prompt(now, &context_section);

        let mut messages: Vec<OrchatMessage> = vec![OrchatMessage {
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
            messages.push(OrchatMessage {
                role: role.to_string(),
                content: msg.content.clone(),
            });
        }

        // 4. Call OpenRouter
        let url = format!("{}/chat/completions", self.api_base);
        let request_body = OrchatRequest {
            model: &self.model,
            messages,
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

        let token_count = or_response.usage.map(|u| u.total_tokens);

        // 5. Parse StructuredResponse with fallback; clean trailing JSON fence from reply.
        let mut structured = parse_response(&raw_content);
        structured.reply = clean_reply(&structured.reply);

        // 6. Append assistant message
        session.messages.push(ChatMessage {
            id: Uuid::new_v4(),
            session_id: session.id,
            role: MessageRole::Assistant,
            content: structured.reply.clone(),
            tool_name: None,
            tool_result: None,
            token_count,
            created_at: chrono::Utc::now(),
        });

        Ok(structured)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn system_prompt(now: chrono::DateTime<chrono::Utc>, context: &str) -> String {
    let user_email =
        std::env::var("TRUSTY_PRIMARY_EMAIL").unwrap_or_else(|_| "bob@matsuoka.com".to_string());
    let user_name = std::env::var("TRUSTY_USER_NAME").unwrap_or_else(|_| "Masa".to_string());
    let context_section = if context.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", context)
    };
    format!(
        r#"You are trusty-izzie, a personal AI assistant with deep knowledge of the user's professional relationships and work context. You run locally on the user's machine.

Today is {}. Current time: {}.

## About Your User
- **Name**: {user_name}
- **Email**: {user_email}
- **Timezone**: America/New_York (EDT, UTC-5)
- You are their personal assistant. Address them by name when appropriate. When they ask who they are or about themselves, use this information.
{context_section}

## My Deployment

I am trusty-izzie v{}, running as macOS launchd services:
- Daemon (com.trusty-izzie.daemon) — event processing, Gmail sync
- API (com.trusty-izzie.api) — REST API on port 3456
- Telegram (com.trusty-izzie.telegram) — Telegram bot on port 3457

I can check my own service status with `check_service_status`, report my version with `get_version`, and file GitHub issues with `submit_github_issue`.

## Email Accounts
I learn from email sent from multiple Google accounts. Use `list_accounts` to see all registered accounts, `add_account` to add a new Google account (I'll return an OAuth URL to visit), or `remove_account` to stop syncing a secondary account.

## What I Can Do
- **macOS Contacts**: I sync with your AddressBook via `sync_contacts`. I know your contact list.

## Available Tools (complete list)
- `check_service_status` — report running status of all trusty-izzie launchd services
- `get_version` — return the current binary version
- `submit_github_issue` — file a GitHub issue via the `gh` CLI
- `schedule_event` — schedule a background task (email_sync, contacts_sync, memory_decay, reminder, agent_run, etc.)
- `cancel_event` — cancel a pending scheduled event by ID
- `list_events` — list scheduled or recent events, optionally filtered by status
- `run_agent` — enqueue a background research agent task
- `list_agents` — list available agent definitions
- `get_agent_task` — get the status and output of an agent task by ID
- `list_accounts` — list all registered Google accounts
- `add_account` — add a Google account (returns OAuth URL; account registered after user consents)
- `remove_account` — deactivate a secondary Google account (stops syncing it)
- `sync_contacts` — queue a macOS AddressBook contacts sync
- `execute_shell_command` — run a bash shell command on this Mac and return stdout/stderr

I do NOT have `read_file`, `write_file`, or `list_directory` tools. To access the file system, use `execute_shell_command` with commands like `ls`, `cat`, etc.

## Shell Access
I can run shell commands on your Mac via `execute_shell_command`. This lets me:
- Read/list files: `ls ~/Downloads`, `cat ~/some/file.txt`
- Run scripts: any bash command
- Check system state: `ps aux | grep something`, `df -h`, etc.
Use this for any scripting or file system tasks.

CRITICAL OUTPUT FORMAT: Your ENTIRE response must be a single raw JSON object. Output ONLY the JSON — no prose before it, no explanation after it, no markdown code fences around it. Start your response with {{ and end with }}.

Required format (output this and nothing else):
{{"reply":"your response to the user (markdown allowed)","memoriesToSave":[],"referencedEntities":[]}}

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
        let prompt = system_prompt(now, "");
        let year = now.format("%Y").to_string();
        assert!(prompt.contains(&year));
    }

    #[test]
    fn test_system_prompt_includes_context_when_nonempty() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(
            now,
            "## Relevant People & Projects\n- Alice (Person): alice",
        );
        assert!(prompt.contains("## Relevant People & Projects"));
    }

    #[test]
    fn test_system_prompt_no_context_section_when_empty() {
        let now = chrono::Utc::now();
        let prompt = system_prompt(now, "");
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
