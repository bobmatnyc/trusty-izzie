//! RAG context assembly for chat completions.

use anyhow::Result;
use std::sync::Arc;

use trusty_memory::MemoryRecaller;
use trusty_models::chat::ChatContext;
use trusty_store::LanceStore;

/// Assembles the retrieval-augmented context for a chat turn.
pub struct ContextAssembler {
    memory_recaller: Option<Arc<MemoryRecaller>>,
    lance: Option<Arc<LanceStore>>,
    memory_limit: usize,
    entity_limit: usize,
}

impl ContextAssembler {
    /// Construct with limits from the chat config. No memory recaller or store attached.
    pub fn new(memory_limit: usize, entity_limit: usize) -> Self {
        Self {
            memory_recaller: None,
            lance: None,
            memory_limit,
            entity_limit,
        }
    }

    /// Attach a `MemoryRecaller` for live memory retrieval.
    pub fn with_memory_recaller(mut self, r: Arc<MemoryRecaller>) -> Self {
        self.memory_recaller = Some(r);
        self
    }

    /// Attach a `LanceStore` for entity RAG retrieval.
    pub fn with_lance(mut self, lance: Arc<LanceStore>) -> Self {
        self.lance = Some(lance);
        self
    }

    /// Build a `ChatContext` from the user's query.
    ///
    /// Retrieves relevant memories (if `MemoryRecaller` attached) and entities
    /// (if `LanceStore` attached) via text search.
    pub async fn assemble(&self, query: &str, _user_id: &str) -> Result<ChatContext> {
        let relevant_memories = match &self.memory_recaller {
            Some(recaller) => recaller.recall(query, self.memory_limit).await?,
            None => vec![],
        };

        let relevant_entities = match &self.lance {
            Some(lance) => lance
                .search_entities_text(query, self.entity_limit)
                .await
                .unwrap_or_default(),
            None => vec![],
        };

        tracing::info!(
            memories = relevant_memories.len(),
            entities = relevant_entities.len(),
            query = query,
            "context assembly complete"
        );

        Ok(ChatContext {
            relevant_memories,
            relevant_entities,
            session_summary: None,
        })
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
