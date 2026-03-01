//! Sync trigger handler.

use axum::{extract::State, http::StatusCode, response::Json};
use serde::Deserialize;
use serde_json::Value;

use crate::state::AppState;

/// Request body for `POST /v1/sync`.
#[derive(Deserialize)]
pub struct TriggerSyncRequest {
    /// Whether to ignore the history cursor and re-scan recent mail.
    pub force: Option<bool>,
}

/// `POST /v1/sync` — request an immediate sync cycle from the daemon.
pub async fn trigger_sync(
    State(_state): State<AppState>,
    Json(_body): Json<TriggerSyncRequest>,
) -> Result<Json<Value>, StatusCode> {
    // Send DaemonCommand::Sync via IPC
    todo!("send DaemonCommand::Sync to daemon over IPC socket")
}
