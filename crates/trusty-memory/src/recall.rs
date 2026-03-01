//! Retrieving relevant memories for a chat query.

use anyhow::Result;
use std::sync::Arc;

use trusty_embeddings::{Embedder, HybridSearcher, SearchMode};
use trusty_models::memory::Memory;
use trusty_store::Store;

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

        let fused = self.searcher.search(
            vector_results,
            keyword_results,
            limit,
            SearchMode::Hybrid,
        )?;

        // TODO: load full Memory objects from store by ID
        let _ = fused;
        todo!("load Memory objects from store by the fused result IDs")
    }
}
