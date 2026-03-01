//! Tool definitions that the LLM can invoke during a chat turn.

use serde::{Deserialize, Serialize};

/// A tool the LLM may request the runtime to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name as passed to/from the LLM.
    pub name: ToolName,
    /// Human-readable description shown to the model.
    pub description: &'static str,
    /// JSON Schema for the tool's input parameters.
    pub parameters: serde_json::Value,
}

/// Enumeration of all available tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    /// Search memories by semantic query.
    SearchMemories,
    /// Look up entities in the knowledge graph.
    SearchEntities,
    /// Fetch all relationships for a named entity.
    GetEntityRelationships,
    /// Save a new memory to the store.
    SaveMemory,
}

/// A parsed tool call request from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Which tool to invoke.
    pub name: ToolName,
    /// The arguments to pass (must match the tool's parameter schema).
    pub arguments: serde_json::Value,
}

/// The result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Which tool produced this result.
    pub name: ToolName,
    /// JSON-serialised result data.
    pub result: serde_json::Value,
    /// Whether the tool succeeded.
    pub success: bool,
    /// Optional error message if `success` is false.
    pub error: Option<String>,
}
