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
        let default_model = "anthropic/claude-sonnet-4-6".to_string();

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

        // 3. Build OpenRouter request
        let system = if let Some(ctx) = &context {
            format!("{instructions}\n\n## Additional Context\n\n{ctx}")
        } else {
            instructions
        };

        let max_tokens = (max_runtime_mins * 1000).clamp(1000, 32000);

        let request_body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": task_description}
            ],
            "max_tokens": max_tokens
        });

        // 4. Call OpenRouter
        let url = format!(
            "{}/chat/completions",
            self.openrouter_base.trim_end_matches('/')
        );
        let client = reqwest::Client::new();
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

        let output = resp_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // 5. Store result
        info!(
            "AgentRun {task_id} completed, output {} chars",
            output.len()
        );
        {
            let sqlite = store.sqlite.clone();
            let tid = task_id.clone();
            tokio::task::spawn_blocking(move || {
                sqlite.update_agent_task(&tid, "done", Some(&output), None)
            })
            .await
            .map_err(|e| TrustyError::Storage(e.to_string()))?
            .map_err(|e| TrustyError::Storage(e.to_string()))?;
        }

        Ok(DispatchResult::Done)
    }
}
