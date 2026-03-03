//! Route registration for the axum router.

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::handlers;
use crate::state::AppState;

/// Build and return the full axum `Router`.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Health
        .route("/health", get(handlers::health::health_check))
        // Chat
        .route("/v1/chat", post(handlers::chat::create_message))
        .route("/v1/chat/sessions", get(handlers::chat::list_sessions))
        .route(
            "/v1/chat/sessions/:session_id",
            get(handlers::chat::get_session),
        )
        .route(
            "/v1/chat/sessions/:session_id",
            delete(handlers::chat::delete_session),
        )
        // Entities
        .route("/v1/entities", get(handlers::entities::list_entities))
        .route(
            "/v1/entities/search",
            get(handlers::entities::search_entities),
        )
        .route("/v1/entities/:id", get(handlers::entities::get_entity))
        .route(
            "/v1/entities/:id/relationships",
            get(handlers::entities::get_relationships),
        )
        // Memories
        .route("/v1/memories", get(handlers::memories::list_memories))
        .route(
            "/v1/memories/search",
            get(handlers::memories::search_memories),
        )
        .route(
            "/v1/memories/:id",
            delete(handlers::memories::delete_memory),
        )
        // Sync
        .route("/v1/sync", post(handlers::sync::trigger_sync))
        // Agents
        .route("/api/agents", get(handlers::agents::list_agents))
        .route("/api/agents/:name", get(handlers::agents::get_agent))
        // Tasks
        .route(
            "/api/tasks",
            get(handlers::agents::list_tasks).post(handlers::agents::create_task),
        )
        .route("/api/tasks/:id", get(handlers::agents::get_task))
        .with_state(state)
}
