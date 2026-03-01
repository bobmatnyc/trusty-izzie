//! RAG context assembly for chat completions.

use anyhow::Result;

use trusty_models::chat::ChatContext;

/// Assembles the retrieval-augmented context for a chat turn.
#[allow(dead_code)]
pub struct ContextAssembler {
    memory_limit: usize,
    entity_limit: usize,
}

impl ContextAssembler {
    /// Construct with limits from the chat config.
    pub fn new(memory_limit: usize, entity_limit: usize) -> Self {
        Self {
            memory_limit,
            entity_limit,
        }
    }

    /// Build a `ChatContext` from the user's query.
    ///
    /// Retrieves relevant memories and entities and bundles them for injection
    /// into the system prompt.
    pub async fn assemble(&self, _query: &str, _user_id: &str) -> Result<ChatContext> {
        todo!("retrieve memories via MemoryRecaller and entities via GraphStore")
    }

    /// Render a `ChatContext` into a markdown-formatted system prompt section.
    pub fn render_context(&self, ctx: &ChatContext) -> String {
        let mut parts = Vec::new();

        if !ctx.relevant_memories.is_empty() {
            parts.push("## Relevant Memories".to_string());
            for mem in &ctx.relevant_memories {
                parts.push(format!("- {}", mem.content));
            }
        }

        if !ctx.relevant_entities.is_empty() {
            parts.push("## Relevant People & Projects".to_string());
            for entity in &ctx.relevant_entities {
                parts.push(format!(
                    "- {} ({:?}): {}",
                    entity.value, entity.entity_type, entity.normalized
                ));
            }
        }

        if let Some(summary) = &ctx.session_summary {
            parts.push("## Earlier in This Conversation".to_string());
            parts.push(summary.clone());
        }

        parts.join("\n")
    }
}
