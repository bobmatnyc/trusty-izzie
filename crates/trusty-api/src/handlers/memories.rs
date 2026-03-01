//! Memory endpoint handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

/// Query parameters for `GET /v1/memories`.
#[derive(Deserialize)]
pub struct ListMemoriesQuery {
    pub category: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// `GET /v1/memories` — list stored memories.
pub async fn list_memories(
    State(_state): State<AppState>,
    Query(_query): Query<ListMemoriesQuery>,
) -> Json<Value> {
    todo!("query LanceDB for memories with optional category filter")
}

/// Query parameters for `GET /v1/memories/search`.
#[derive(Deserialize)]
pub struct SearchMemoriesQuery {
    pub q: String,
    pub limit: Option<usize>,
}

/// `GET /v1/memories/search` — semantic memory search.
pub async fn search_memories(
    State(_state): State<AppState>,
    Query(_query): Query<SearchMemoriesQuery>,
) -> Json<Value> {
    todo!("call MemoryRecaller with hybrid search")
}

/// `DELETE /v1/memories/:id` — delete a memory by ID.
pub async fn delete_memory(
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> StatusCode {
    todo!("delete memory from LanceDB by UUID")
}
