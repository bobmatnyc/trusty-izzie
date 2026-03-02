//! Storage layer for trusty-izzie.
//!
//! Three complementary backends are composed into a single `Store`:
//! - **LanceDB** — columnar vector store for semantic similarity search.
//! - **Kuzu** — embedded graph database for entity/relationship traversal.
//! - **SQLite** — relational store for auth tokens, config, and sync cursors.

pub mod graph;
pub mod lance;
pub mod sqlite;

pub use graph::GraphStore;
pub use lance::LanceStore;
pub use sqlite::SqliteStore;

use anyhow::Result;
use std::path::Path;

/// The unified storage handle. Pass this (wrapped in `Arc`) through the app.
pub struct Store {
    /// Vector similarity search over memories and entities.
    pub lance: LanceStore,
    /// Knowledge graph of entities and relationships.
    pub graph: GraphStore,
    /// Auth tokens, history cursors, and application config.
    pub sqlite: SqliteStore,
}

impl Store {
    /// Open all three storage backends rooted at `data_dir`.
    ///
    /// `user_id` is used to tag LanceDB records (single-tenant: one value per instance).
    ///
    /// Directories are created automatically if they do not exist.
    pub async fn open(data_dir: &Path, user_id: &str) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        let lance = LanceStore::open(&data_dir.join("lance"), user_id).await?;
        let graph = GraphStore::open(&data_dir.join("kuzu"))?;
        let sqlite = SqliteStore::open(&data_dir.join("trusty.db"))?;

        Ok(Self {
            lance,
            graph,
            sqlite,
        })
    }
}
