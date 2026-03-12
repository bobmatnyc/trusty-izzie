//! Core `Skill` trait that every skill crate must implement.

use async_trait::async_trait;
use serde_json::Value;

/// One tool a skill exposes to the LLM.
#[derive(Debug, Clone)]
pub struct SkillTool {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters: Value,
}

/// An incoming tool call from the LLM destined for a skill.
#[derive(Debug, Clone)]
pub struct SkillToolCall {
    pub name: String,
    pub arguments: Value,
}

/// The result of executing a skill tool.
pub type SkillResult = anyhow::Result<String>;

/// Core trait every skill crate must implement.
#[async_trait]
pub trait Skill: Send + Sync + 'static {
    /// Unique kebab-case identifier, e.g. "metro-north", "weather".
    fn name(&self) -> &str;

    /// One-line human description, shown in the skills directory.
    fn description(&self) -> &str;

    /// Tools this skill exposes to the LLM.
    fn tools(&self) -> Vec<SkillTool>;

    /// Optional markdown injected into the system prompt under "## Skills".
    /// Return `None` if the skill needs no prompt context.
    fn system_prompt_contribution(&self) -> Option<String> {
        None
    }

    /// Execute a tool call. Return `None` if this skill doesn't handle `call.name`.
    async fn execute(&self, call: &SkillToolCall) -> Option<SkillResult>;

    /// Whether this skill can also run as a standalone MCP server.
    fn mcp_capable(&self) -> bool {
        false
    }
}
