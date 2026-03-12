use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct AgentRunHandler {
    agents_dir: PathBuf,
    openrouter_base: String,
    openrouter_api_key: String,
}

impl AgentRunHandler {
    pub fn new(agents_dir: PathBuf, openrouter_base: String, openrouter_api_key: String) -> Self {
        Self {
            agents_dir,
            openrouter_base,
            openrouter_api_key,
        }
    }

    /// Parse YAML front-matter from an agent MD file.
    /// Format: optional `---\nkey: value\n---\n` at start of file.
    /// Returns (model, max_runtime_mins, description, instructions_body).
    fn parse_agent_file(&self, content: &str) -> (String, u32, String, String) {
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
}

fn agent_tool_definitions() -> serde_json::Value {
    serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the web using Brave Search API. Use for current information, news, facts you don't know.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "count": { "type": "integer", "default": 5, "description": "Number of results (1-10)" }
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "fetch_page",
                "description": "Fetch and read a web page as plain text. Use after web_search to get full content from a URL.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL to fetch" },
                        "max_chars": { "type": "integer", "default": 4000, "description": "Max characters to return" }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "execute_shell_command",
                "description": "Execute a shell command and return stdout. Use sparingly for data processing tasks.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to execute" }
                    },
                    "required": ["command"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "tavily_search",
                "description": "Deep web search optimized for research agents. Returns full page content, not just snippets. Prefer this over web_search for research tasks requiring detailed information.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Research query" },
                        "search_depth": { "type": "string", "enum": ["basic", "advanced"], "default": "basic" },
                        "max_results": { "type": "integer", "default": 5 }
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "search_memories",
                "description": "Search the user's stored memories and personal context by semantic query.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "What to search for" },
                        "limit": { "type": "integer", "default": 5 }
                    },
                    "required": ["query"]
                }
            }
        }
    ])
}

async fn execute_agent_tool(name: &str, args: &serde_json::Value, _store: &Arc<Store>) -> String {
    match name {
        "web_search" => {
            let query = args["query"].as_str().unwrap_or("");
            let count = args["count"].as_u64().unwrap_or(5).min(10);
            let api_key = match std::env::var("BRAVE_SEARCH_API_KEY") {
                Ok(k) => k,
                Err(_) => return "Error: BRAVE_SEARCH_API_KEY not set".into(),
            };
            let url = format!(
                "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
                urlencoding::encode(query),
                count
            );
            let client = reqwest::Client::new();
            match client
                .get(&url)
                .header("Accept", "application/json")
                .header("X-Subscription-Token", &api_key)
                .send()
                .await
            {
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Ok(json) => {
                        let results = json["web"]["results"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .map(|r| {
                                        format!(
                                            "**{}**\n{}\n{}",
                                            r["title"].as_str().unwrap_or(""),
                                            r["description"].as_str().unwrap_or(""),
                                            r["url"].as_str().unwrap_or("")
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n\n")
                            })
                            .unwrap_or_default();
                        if results.is_empty() {
                            "No results found.".into()
                        } else {
                            results
                        }
                    }
                    Err(e) => format!("Parse error: {e}"),
                },
                Err(e) => format!("Request error: {e}"),
            }
        }
        "tavily_search" => {
            let query = args["query"].as_str().unwrap_or("");
            let depth = args["search_depth"].as_str().unwrap_or("basic");
            let max_results = args["max_results"].as_u64().unwrap_or(5);
            let api_key = match std::env::var("TAVILY_API_KEY") {
                Ok(k) => k,
                Err(_) => return "Error: TAVILY_API_KEY not configured".into(),
            };
            let client = reqwest::Client::new();
            match client
                .post("https://api.tavily.com/search")
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "api_key": api_key,
                    "query": query,
                    "search_depth": depth,
                    "max_results": max_results,
                    "include_answer": true,
                    "include_raw_content": false
                }))
                .send()
                .await
            {
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Ok(json) => {
                        let mut parts = vec![];
                        if let Some(answer) = json["answer"].as_str() {
                            if !answer.is_empty() {
                                parts.push(format!("**Summary**: {answer}"));
                            }
                        }
                        if let Some(results) = json["results"].as_array() {
                            for r in results {
                                let title = r["title"].as_str().unwrap_or("");
                                let content = r["content"].as_str().unwrap_or("");
                                let url = r["url"].as_str().unwrap_or("");
                                parts.push(format!("**{title}**\n{content}\n{url}"));
                            }
                        }
                        if parts.is_empty() {
                            "No results.".into()
                        } else {
                            parts.join("\n\n")
                        }
                    }
                    Err(e) => format!("Parse error: {e}"),
                },
                Err(e) => format!("Request error: {e}"),
            }
        }
        "fetch_page" => {
            let url = args["url"].as_str().unwrap_or("");
            let max_chars = args["max_chars"].as_u64().unwrap_or(4000) as usize;
            let client = reqwest::Client::builder()
                .user_agent("TrustyIzzie-Agent/1.0")
                .build()
                .unwrap_or_default();
            match client.get(url).send().await {
                Ok(resp) => match resp.text().await {
                    Ok(html) => {
                        // Strip HTML tags and script blocks
                        let mut out = String::with_capacity(html.len());
                        let mut in_tag = false;
                        let mut in_script = false;
                        let lower = html.to_lowercase();
                        let mut i = 0;
                        let bytes = html.as_bytes();
                        while i < bytes.len() {
                            if !in_script && i + 7 < bytes.len() && &lower[i..i + 7] == "<script" {
                                in_script = true;
                            }
                            if in_script {
                                if i + 9 < bytes.len() && &lower[i..i + 9] == "</script>" {
                                    in_script = false;
                                    i += 9;
                                } else {
                                    i += 1;
                                }
                                continue;
                            }
                            if bytes[i] == b'<' {
                                in_tag = true;
                            } else if bytes[i] == b'>' {
                                in_tag = false;
                                out.push(' ');
                            } else if !in_tag {
                                out.push(bytes[i] as char);
                            }
                            i += 1;
                        }
                        let text: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
                        if text.len() > max_chars {
                            text[..max_chars].to_string()
                        } else {
                            text
                        }
                    }
                    Err(e) => format!("Read error: {e}"),
                },
                Err(e) => format!("Fetch error: {e}"),
            }
        }
        "execute_shell_command" => {
            let command = args["command"].as_str().unwrap_or("");
            match tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .output()
                .await
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if stdout.is_empty() && !stderr.is_empty() {
                        format!("stderr: {stderr}")
                    } else {
                        stdout.to_string()
                    }
                }
                Err(e) => format!("Command error: {e}"),
            }
        }
        "search_memories" => {
            // LanceStore::search_memories requires a pre-computed embedding vector;
            // no plain-text search wrapper exists at the Store level yet.
            let _ = args;
            "Memory search not available from agent context.".into()
        }
        other => format!("Unknown tool: {other}"),
    }
}

async fn push_telegram_notification(store: &Arc<Store>, agent_name: &str, output: &str) {
    let token = match std::env::var("TELEGRAM_BOT_TOKEN") {
        Ok(t) => t,
        Err(_) => return,
    };

    let chat_id_str = match store
        .sqlite
        .get_config("telegram_primary_chat_id")
        .ok()
        .flatten()
    {
        Some(id) => id,
        None => return,
    };
    let chat_id: i64 = match chat_id_str.parse() {
        Ok(id) => id,
        Err(_) => return,
    };

    let preview = if output.len() > 800 {
        format!("{}…", &output[..800])
    } else {
        output.to_string()
    };

    let text = format!("🤖 *Agent: {}* finished\n\n{}", agent_name, preview);

    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let client = reqwest::Client::new();
    let _ = client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "Markdown"
        }))
        .send()
        .await;
    // Ignore errors — notification is best-effort
}

#[async_trait]
impl EventHandler for AgentRunHandler {
    fn event_type(&self) -> EventType {
        EventType::AgentRun
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let (agent_name, task_description, context) = match &event.payload {
            EventPayload::AgentRun {
                agent_name,
                task_description,
                context,
            } => (
                agent_name.clone(),
                task_description.clone(),
                context.clone(),
            ),
            _ => return Err(TrustyError::Storage("wrong payload type".into())),
        };

        // 1. Read agent definition file
        let agent_path = self.agents_dir.join(format!("{}.md", agent_name));
        let content = tokio::fs::read_to_string(&agent_path).await.map_err(|e| {
            TrustyError::Storage(format!("agent file not found: {agent_name}: {e}"))
        })?;

        let (model, max_runtime_mins, _description, instructions) = self.parse_agent_file(&content);

        // 2. Create task record
        let task_id = uuid::Uuid::new_v4().to_string();
        let event_id_str = event.id.to_string();
        {
            let sqlite = store.sqlite.clone();
            let tid = task_id.clone();
            let aname = agent_name.clone();
            let tdesc = task_description.clone();
            let m = model.clone();
            let eid = event_id_str.clone();
            tokio::task::spawn_blocking(move || {
                sqlite.create_agent_task(&tid, &aname, &tdesc, Some(&m), Some(&eid))
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;
        }
        {
            let sqlite = store.sqlite.clone();
            let tid = task_id.clone();
            tokio::task::spawn_blocking(move || {
                sqlite.update_agent_task(&tid, "running", None, None)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;
        }

        // 3. Build system prompt
        let system = if let Some(ctx) = &context {
            format!("{instructions}\n\n## Additional Context\n\n{ctx}")
        } else {
            instructions
        };

        let max_tokens = (max_runtime_mins * 1000).clamp(1000, 32000);

        // 4. Tool loop
        let url = format!(
            "{}/chat/completions",
            self.openrouter_base.trim_end_matches('/')
        );
        let client = reqwest::Client::new();
        let tools = agent_tool_definitions();

        let mut messages: Vec<serde_json::Value> =
            vec![serde_json::json!({"role": "user", "content": task_description})];

        let max_iterations = 10u32;
        let mut final_output = String::new();

        for iteration in 0..max_iterations {
            let request_body = serde_json::json!({
                "model": model,
                "max_tokens": max_tokens,
                "system": system,
                "messages": messages,
                "tools": tools,
                "tool_choice": "auto"
            });

            let response = client
                .post(&url)
                .header(
                    "Authorization",
                    format!("Bearer {}", self.openrouter_api_key),
                )
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await
                .map_err(|e| TrustyError::Http(format!("OpenRouter request failed: {e}")))?;

            if !response.status().is_success() {
                let err = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown error".into());
                error!("AgentRun {task_id}: OpenRouter error: {err}");
                let sqlite = store.sqlite.clone();
                let tid = task_id.clone();
                let err_clone = err.clone();
                tokio::task::spawn_blocking(move || {
                    sqlite.update_agent_task(&tid, "error", None, Some(&err_clone))
                })
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?;
                return Err(TrustyError::Http(format!("OpenRouter error: {err}")));
            }

            let resp_json: serde_json::Value = response
                .json()
                .await
                .map_err(|e| TrustyError::Serialization(format!("parse error: {e}")))?;

            let choice = &resp_json["choices"][0];
            let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop");
            let message = &choice["message"];

            // Add assistant message to history
            messages.push(message.clone());

            if finish_reason == "tool_calls" {
                let tool_calls = message["tool_calls"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                for tc in &tool_calls {
                    let tool_name = tc["function"]["name"].as_str().unwrap_or("");
                    let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: serde_json::Value =
                        serde_json::from_str(args_str).unwrap_or_default();

                    info!("Agent {task_id} iter {iteration}: calling tool {tool_name}");

                    let result = execute_agent_tool(tool_name, &args, store).await;

                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tc["id"],
                        "content": result
                    }));
                }
            } else {
                // Terminal response — extract text
                final_output = message["content"]
                    .as_str()
                    .map(|s| s.to_string())
                    .or_else(|| {
                        message["content"].as_array().and_then(|arr| {
                            arr.iter()
                                .find(|b| b["type"] == "text")
                                .and_then(|b| b["text"].as_str())
                                .map(|s| s.to_string())
                        })
                    })
                    .unwrap_or_default();
                break;
            }
        }

        if final_output.is_empty() {
            final_output = "Agent completed but produced no text output.".to_string();
        }

        // 5. Store result
        info!(
            "AgentRun {task_id} completed, output {} chars",
            final_output.len()
        );
        {
            let sqlite = store.sqlite.clone();
            let tid = task_id.clone();
            let out = final_output.clone();
            tokio::task::spawn_blocking(move || {
                sqlite.update_agent_task(&tid, "done", Some(&out), None)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;
        }

        // 6. Telegram notification (best-effort)
        push_telegram_notification(store, &agent_name, &final_output).await;

        Ok(DispatchResult::Done)
    }
}
