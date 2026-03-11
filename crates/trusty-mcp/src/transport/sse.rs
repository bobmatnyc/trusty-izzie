//! HTTP SSE transport for MCP.
//!
//! Endpoints:
//!   GET  /mcp/sse     — open an SSE stream; receives an `endpoint` event with the POST URL
//!   POST /mcp/message — send a JSON-RPC request, receive a JSON-RPC response

use std::convert::Infallible;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self, Stream};
use serde::Deserialize;
use tower_http::trace::TraceLayer;
use tracing::info;
use uuid::Uuid;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::server::McpServer;

#[derive(Clone)]
struct AppState {
    server: Arc<McpServer>,
}

#[derive(Deserialize)]
struct SessionParams {
    #[allow(dead_code)]
    session: Option<String>,
}

pub async fn run_http(server: Arc<McpServer>, port: u16) -> Result<()> {
    let state = AppState { server };
    let app = Router::new()
        .route("/mcp/sse", get(sse_handler))
        .route("/mcp/message", post(message_handler))
        .route("/message", post(message_handler))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let addr = format!("127.0.0.1:{port}");
    info!(address = %addr, "starting trusty-mcp HTTP server");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn sse_handler(
    State(_state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let session = Uuid::new_v4().to_string();
    let endpoint_url = format!("/mcp/message?session={session}");
    let stream = stream::iter(vec![Ok(Event::default()
        .event("endpoint")
        .data(endpoint_url))]);
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn message_handler(
    State(state): State<AppState>,
    Query(_params): Query<SessionParams>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let resp: JsonRpcResponse = state.server.handle(req).await;
    Json(resp)
}
