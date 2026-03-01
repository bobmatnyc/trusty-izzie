//! LanceDB vector store for memory and entity embeddings.

use anyhow::Result;
use std::path::Path;

/// Handle to the LanceDB vector store.
pub struct LanceStore {
    /// The LanceDB connection handle.
    _connection: lancedb::Connection,
}

impl LanceStore {
    /// Open (or create) a LanceDB database at `path`.
    pub async fn open(path: &Path) -> Result<Self> {
        let uri = path.to_string_lossy();
        let connection = lancedb::connect(&uri).execute().await?;
        Ok(Self {
            _connection: connection,
        })
    }

    /// Upsert a memory embedding vector.
    ///
    /// The vector must match the dimensionality configured for the embedder.
    pub async fn upsert_memory(&self, _id: &str, _embedding: &[f32], _content: &str) -> Result<()> {
        todo!("implement memory vector upsert in LanceDB")
    }

    /// Perform approximate nearest-neighbour search for memories.
    ///
    /// Returns at most `limit` `(id, distance)` pairs.
    pub async fn search_memories(
        &self,
        _query_embedding: &[f32],
        _limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        todo!("implement memory ANN search in LanceDB")
    }

    /// Delete a memory by ID.
    pub async fn delete_memory(&self, _id: &str) -> Result<()> {
        todo!("implement memory deletion in LanceDB")
    }
}
