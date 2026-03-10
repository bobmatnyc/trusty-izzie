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
        .route("/chat", post(handlers::chat::create_message))
        .route("/chat/sessions", get(handlers::chat::list_sessions))
        .route(
            "/chat/sessions/:session_id",
            get(handlers::chat::get_session),
        )
        .route(
            "/chat/sessions/:session_id",
            delete(handlers::chat::delete_session),
        )
        // Entities
        .route("/entities", get(handlers::entities::list_entities))
        .route("/entities/search", get(handlers::entities::search_entities))
        .route("/entities/:id", get(handlers::entities::get_entity))
        .route(
            "/entities/:id/relationships",
            get(handlers::entities::get_relationships),
        )
        // Memories
        .route("/memories", get(handlers::memories::list_memories))
        .route("/memories/search", get(handlers::memories::search_memories))
        .route("/memories/:id", delete(handlers::memories::delete_memory))
        // Sync
        .route("/sync", post(handlers::sync::trigger_sync))
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
