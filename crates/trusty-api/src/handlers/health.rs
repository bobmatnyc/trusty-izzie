//! Health check handler.

use axum::response::Json;
use serde_json::{json, Value};

/// `GET /health` — returns a simple JSON health response.
pub async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "trusty-api" }))
}
