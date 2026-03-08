//! Agent and task endpoint handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use trusty_models::{EventPayload, EventType};

use crate::state::AppState;

// ── Agent listing ─────────────────────────────────────────────────────────────

/// `GET /api/agents` — list all agent definitions.
pub async fn list_agents(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    let agents_dir = state.agents_dir.clone();
    let agents = tokio::task::spawn_blocking(move || {
        let mut agents: Vec<serde_json::Value> = Vec::new();
        let entries = std::fs::read_dir(&agents_dir).map_err(|e| {
            tracing::warn!("cannot read agents dir {:?}: {e}", agents_dir);
        });
        let entries = match entries {
            Ok(e) => e,
            Err(_) => return agents,
        };
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
            let (model, max_runtime_mins, description) = parse_front_matter(&content);
            agents.push(serde_json::json!({
                "name": stem,
                "model": model,
                "description": description,
                "max_runtime_mins": max_runtime_mins,
            }));
        }
        agents
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!(agents)))
}

/// `GET /api/agents/{name}` — single agent definition + last 10 tasks.
pub async fn get_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let agents_dir = state.agents_dir.clone();
    let sqlite = state.sqlite.clone();
    let name_clone = name.clone();

    let result = tokio::task::spawn_blocking(move || {
        let path = agents_dir.join(format!("{}.md", name_clone));
        let content = std::fs::read_to_string(&path).map_err(|_| StatusCode::NOT_FOUND)?;
        let (model, max_runtime_mins, description) = parse_front_matter(&content);

        let tasks = sqlite
            .list_agent_tasks(None, 10)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.agent_name == name_clone)
            .collect::<Vec<_>>();

        Ok::<_, StatusCode>(serde_json::json!({
            "name": name_clone,
            "model": model,
            "description": description,
            "max_runtime_mins": max_runtime_mins,
            "recent_tasks": tasks,
        }))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(result))
}

// ── Task listing ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TasksQuery {
    pub status: Option<String>,
}

/// `GET /api/tasks` — all agent tasks, optional ?status=running filter.
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<TasksQuery>,
) -> Result<Json<Value>, StatusCode> {
    let sqlite = state.sqlite.clone();
    let status = query.status.clone();
    let tasks =
        tokio::task::spawn_blocking(move || sqlite.list_agent_tasks(status.as_deref(), 100))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!(tasks)))
}

/// `GET /api/tasks/{id}` — task detail + full output.
pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let sqlite = state.sqlite.clone();
    let task = tokio::task::spawn_blocking(move || sqlite.get_agent_task(&id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match task {
        None => Err(StatusCode::NOT_FOUND),
        Some(t) => Ok(Json(
            serde_json::to_value(t).unwrap_or(serde_json::json!({})),
        )),
    }
}

// ── Task creation ─────────────────────────────────────────────────────────────

/// Request body for `POST /api/tasks`.
#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub agent_name: String,
    pub task_description: String,
    pub context: Option<String>,
}

/// Response body for `POST /api/tasks`.
#[derive(Serialize)]
pub struct CreateTaskResponse {
    pub task_id: String,
}

/// `POST /api/tasks` — enqueue an AgentRun event and return the task_id.
pub async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<Json<CreateTaskResponse>, StatusCode> {
    if body.agent_name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if body.task_description.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let sqlite = state.sqlite.clone();
    let agent_name = body.agent_name.clone();
    let task_description = body.task_description.clone();
    let context = body.context.clone();

    let task_id = tokio::task::spawn_blocking(move || {
        let payload = EventPayload::AgentRun {
            agent_name,
            task_description,
            context,
        };
        let now = chrono::Utc::now().timestamp();
        sqlite.enqueue_event(
            &EventType::AgentRun,
            &payload,
            now,
            EventType::AgentRun.default_priority(),
            EventType::AgentRun.default_max_retries(),
            "api",
            None,
        )
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(CreateTaskResponse { task_id }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse YAML front-matter from agent MD content.
/// Returns (model, max_runtime_mins, description).
fn parse_front_matter(content: &str) -> (String, u32, String) {
    let default_model = "anthropic/claude-sonnet-4-6".to_string();
    if !content.starts_with("---") {
        return (default_model, 30, String::new());
    }
    let rest = &content[3..];
    let end = rest.find("\n---").unwrap_or(rest.len());
    let front_matter = &rest[..end];
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
    (model, max_runtime_mins, description)
}
