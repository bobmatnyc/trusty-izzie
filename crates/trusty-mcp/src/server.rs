//! McpServer: JSON-RPC dispatch for all MCP methods.

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::{debug, warn};
use trusty_chat::engine::ChatEngine;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse, ToolResult};
use crate::tools;

pub struct McpServer {
    engine: Arc<ChatEngine>,
}

impl McpServer {
    pub fn new(engine: Arc<ChatEngine>) -> Self {
        Self { engine }
    }

    pub async fn handle(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        debug!(method = %req.method, "MCP request");
        match req.method.as_str() {
            "initialize" => self.handle_initialize(req.id),
            "initialized" => JsonRpcResponse::ok(req.id, json!({})),
            "tools/list" => self.handle_tools_list(req.id),
            "tools/call" => self.handle_tools_call(req.id, req.params).await,
            "ping" => JsonRpcResponse::ok(req.id, json!({})),
            other => {
                warn!(method = %other, "unknown MCP method");
                JsonRpcResponse::method_not_found(req.id)
            }
        }
    }

    fn handle_initialize(&self, id: Option<Value>) -> JsonRpcResponse {
        JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "trusty-izzie",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )
    }

    fn handle_tools_list(&self, id: Option<Value>) -> JsonRpcResponse {
        JsonRpcResponse::ok(id, json!({ "tools": tools::all_tools() }))
    }

    async fn handle_tools_call(&self, id: Option<Value>, params: Option<Value>) -> JsonRpcResponse {
        let params = match params {
            Some(p) => p,
            None => return JsonRpcResponse::invalid_params(id, "Missing params"),
        };
        let name = match params["name"].as_str() {
            Some(n) => n.to_string(),
            None => return JsonRpcResponse::invalid_params(id, "Missing tool name"),
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let tool_result = match tools::dispatch(&self.engine, &name, &arguments).await {
            Ok(text) => ToolResult::text(text),
            Err(e) => {
                warn!(tool = %name, error = %e, "tool call failed");
                ToolResult::error(e.to_string())
            }
        };

        match serde_json::to_value(tool_result) {
            Ok(v) => JsonRpcResponse::ok(id, v),
            Err(e) => JsonRpcResponse::err(id, -32603, format!("Internal error: {e}")),
        }
    }
}
