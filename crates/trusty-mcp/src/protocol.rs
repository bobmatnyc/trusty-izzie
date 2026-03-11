//! JSON-RPC 2.0 types for the MCP protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }

    pub fn method_not_found(id: Option<Value>) -> Self {
        Self::err(id, -32601, "Method not found")
    }

    pub fn parse_error() -> Self {
        Self::err(None, -32700, "Parse error")
    }

    pub fn invalid_params(id: Option<Value>, msg: impl Into<String>) -> Self {
        Self::err(id, -32602, msg)
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

/// A tool definition for the tools/list response.
#[derive(Debug, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// The result envelope for a tools/call response.
#[derive(Debug, Serialize)]
pub struct ToolResult {
    pub content: Vec<Content>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            is_error: true,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Content {
    #[serde(rename = "type")]
    pub type_: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Content {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            type_: "text",
            text: Some(text.into()),
        }
    }
}
