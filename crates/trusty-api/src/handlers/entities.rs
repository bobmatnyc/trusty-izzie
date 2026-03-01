//! Entity endpoint handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

/// Query parameters for `GET /v1/entities`.
#[derive(Deserialize)]
pub struct ListEntitiesQuery {
    pub entity_type: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// `GET /v1/entities` — list entities with optional type filter.
pub async fn list_entities(
    State(_state): State<AppState>,
    Query(_query): Query<ListEntitiesQuery>,
) -> Json<Value> {
    todo!("query GraphStore for entities with optional type filter")
}

/// Query parameters for `GET /v1/entities/search`.
#[derive(Deserialize)]
pub struct SearchEntitiesQuery {
    pub q: String,
    pub limit: Option<usize>,
}

/// `GET /v1/entities/search` — semantic + BM25 entity search.
pub async fn search_entities(
    State(_state): State<AppState>,
    Query(_query): Query<SearchEntitiesQuery>,
) -> Json<Value> {
    todo!("hybrid search over entity embeddings and BM25 index")
}

/// `GET /v1/entities/:id` — retrieve a single entity by ID.
pub async fn get_entity(
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    todo!("look up entity by UUID in GraphStore")
}

/// `GET /v1/entities/:id/relationships` — fetch all relationships for an entity.
pub async fn get_relationships(
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    todo!("traverse entity relationships in GraphStore")
}
