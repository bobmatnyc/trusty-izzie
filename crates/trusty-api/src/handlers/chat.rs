//! Chat endpoint handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

/// Request body for `POST /v1/chat`.
#[derive(Deserialize)]
pub struct CreateMessageRequest {
    /// The user's message text.
    pub message: String,
    /// Optional session ID to continue an existing conversation.
    pub session_id: Option<Uuid>,
}

/// Response body for `POST /v1/chat`.
#[derive(Serialize)]
pub struct CreateMessageResponse {
    pub session_id: Uuid,
    pub reply: String,
}

/// `POST /v1/chat` — send a message and receive a reply.
pub async fn create_message(
    State(_state): State<AppState>,
    Json(_body): Json<CreateMessageRequest>,
) -> Result<Json<CreateMessageResponse>, StatusCode> {
    todo!("route message through ChatEngine and return StructuredResponse.reply")
}

/// `GET /v1/chat/sessions` — list all sessions for the authenticated user.
pub async fn list_sessions(State(_state): State<AppState>) -> Json<Value> {
    todo!("load session list from SessionManager")
}

/// `GET /v1/chat/sessions/:session_id` — retrieve a single session with messages.
pub async fn get_session(
    State(_state): State<AppState>,
    Path(_session_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    todo!("load session by ID from SessionManager")
}

/// `DELETE /v1/chat/sessions/:session_id` — delete a session.
pub async fn delete_session(
    State(_state): State<AppState>,
    Path(_session_id): Path<Uuid>,
) -> StatusCode {
    todo!("delete session from SQLite")
}
