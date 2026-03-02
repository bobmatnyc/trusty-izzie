//! Retrieving relevant memories for a chat query.

use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tracing::warn;

use trusty_embeddings::{Embedder, HybridSearcher, SearchMode};
use trusty_models::memory::Memory;
use trusty_store::Store;

use crate::decay::{rank_memories, RankedMemory};

/// Retrieves memories relevant to a query using hybrid search.
pub struct MemoryRecaller {
    store: Arc<Store>,
    embedder: Arc<Embedder>,
    searcher: HybridSearcher,
}

impl MemoryRecaller {
    /// Construct with shared handles to the store, embedder, and hybrid searcher.
    pub fn new(store: Arc<Store>, embedder: Arc<Embedder>) -> Self {
        Self {
            store,
            embedder,
            searcher: HybridSearcher::default(),
        }
    }

    /// Retrieve the top-`limit` memories most relevant to `query`.
    ///
    /// Uses hybrid search: vector ANN over LanceDB + BM25 over Tantivy,
    /// fused with RRF (k=60, alpha=0.7).
    pub async fn recall(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        let query_embedding = self.embedder.embed(query)?;

        // Vector search
        let vector_hits = self
            .store
            .lance
            .search_memories(&query_embedding, limit * 2)
            .await?;

        let vector_results: Vec<(String, String)> = vector_hits
            .into_iter()
            .map(|(id, _dist)| (id, String::new()))
            .collect();

        // BM25 search — TODO: wire tantivy once full-text index is integrated in store
        let keyword_results: Vec<(String, String)> = vec![];

        let fused =
            self.searcher
                .search(vector_results, keyword_results, limit, SearchMode::Hybrid)?;

        // Load full Memory objects by fused result IDs, preserving order
        let mut memories = Vec::with_capacity(fused.len());
        for result in &fused {
            match self.store.lance.get_memory_by_id(&result.id).await? {
                Some(mut memory) => {
                    // Update access metadata if embedding is available for re-upsert
                    memory.access_count = memory.access_count.saturating_add(1);
                    memory.last_accessed = Some(Utc::now());
                    match memory.embedding.clone() {
                        Some(emb) => {
                            if let Err(e) = self.store.lance.upsert_memory(&memory, emb).await {
                                warn!(
                                    "failed to update access metadata for memory {}: {e}",
                                    memory.id
                                );
                            }
                        }
                        None => {
                            warn!(
                                "memory {} has no embedding; skipping access update",
                                memory.id
                            );
                        }
                    }
                    memories.push(memory);
                }
                None => {
                    // ID returned by search but not found — stale index entry, skip silently
                }
            }
        }

        Ok(memories)
    }

    /// Like `recall()`, but returns memories sorted by composite score
    /// (decay strength × relevance × importance).
    pub async fn recall_ranked(&self, query: &str, limit: usize) -> Result<Vec<RankedMemory>> {
        let query_embedding = self.embedder.embed(query)?;

        let vector_hits = self
            .store
            .lance
            .search_memories(&query_embedding, limit * 2)
            .await?;

        // Preserve (id, distance) for ranking
        let id_distance: Vec<(String, f32)> = vector_hits;

        let vector_results: Vec<(String, String)> = id_distance
            .iter()
            .map(|(id, _)| (id.clone(), String::new()))
            .collect();

        let keyword_results: Vec<(String, String)> = vec![];

        let fused = self.searcher.search(
            vector_results,
            keyword_results,
            limit * 2,
            SearchMode::Hybrid,
        )?;

        // Build a distance lookup from fused IDs back to original distances
        let distance_map: std::collections::HashMap<&str, f32> = id_distance
            .iter()
            .map(|(id, dist)| (id.as_str(), *dist))
            .collect();

        // Load Memory objects and pair with distances
        let mut pairs: Vec<(Memory, f32)> = Vec::with_capacity(fused.len());
        for result in &fused {
            if let Some(memory) = self.store.lance.get_memory_by_id(&result.id).await? {
                let distance = distance_map.get(result.id.as_str()).copied().unwrap_or(0.0);
                pairs.push((memory, distance));
            }
        }

        let ranked = rank_memories(pairs, Utc::now());
        Ok(ranked.into_iter().take(limit).collect())
    }
}
