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

/// Request body for `POST /chat`.
#[derive(Deserialize)]
pub struct CreateMessageRequest {
    /// The user's message text.
    pub message: String,
    /// Optional session ID to continue an existing conversation.
    pub session_id: Option<Uuid>,
}

/// Response body for `POST /chat`.
#[derive(Serialize)]
pub struct CreateMessageResponse {
    pub session_id: Uuid,
    pub reply: String,
}

/// `POST /chat` — send a message and receive a reply.
pub async fn create_message(
    State(_state): State<AppState>,
    Json(_body): Json<CreateMessageRequest>,
) -> Result<Json<CreateMessageResponse>, StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

/// `GET /chat/sessions` — list all sessions for the authenticated user.
pub async fn list_sessions(State(_state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

/// `GET /chat/sessions/:session_id` — retrieve a single session with messages.
pub async fn get_session(
    State(_state): State<AppState>,
    Path(_session_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

/// `DELETE /chat/sessions/:session_id` — delete a session.
pub async fn delete_session(
    State(_state): State<AppState>,
    Path(_session_id): Path<Uuid>,
) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}
