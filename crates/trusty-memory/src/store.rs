//! Persisting memories to the store layer.

use anyhow::Result;
use std::sync::Arc;
use uuid::Uuid;

use trusty_embeddings::Embedder;
use trusty_models::memory::{Memory, MemoryCategory};
use trusty_store::Store;

/// Handles writing new memories to the vector store and graph.
pub struct MemoryStore {
    store: Arc<Store>,
    embedder: Arc<Embedder>,
}

impl MemoryStore {
    /// Construct with shared handles to the store and embedder.
    pub fn new(store: Arc<Store>, embedder: Arc<Embedder>) -> Self {
        Self { store, embedder }
    }

    /// Embed and persist a new memory.
    ///
    /// The embedding is generated synchronously (fastembed is CPU-only).
    /// For high-throughput scenarios, consider offloading to `spawn_blocking`.
    pub async fn save(
        &self,
        user_id: &str,
        content: &str,
        category: MemoryCategory,
        related_entities: Vec<String>,
        importance: f32,
        source_id: Option<String>,
    ) -> Result<Memory> {
        let embedding = self.embedder.embed(content)?;
        let now = chrono::Utc::now();
        let id = Uuid::new_v4();

        let memory = Memory {
            id,
            user_id: user_id.to_string(),
            category,
            content: content.to_string(),
            embedding: Some(embedding.clone()),
            related_entities,
            source_id,
            importance,
            access_count: 0,
            last_accessed: None,
            created_at: now,
            updated_at: now,
        };

        // Persist to LanceDB
        self.store.lance.upsert_memory(&memory, embedding).await?;

        Ok(memory)
    }
}
