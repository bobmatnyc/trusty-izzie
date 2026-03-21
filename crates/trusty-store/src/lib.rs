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
pub use sqlite::{InboxRule, SqliteStore};

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::warn;

/// Lazy wrapper around `GraphStore` that defers opening KuzuDB until first use.
///
/// If the database is locked by another process (e.g. trusty-telegram), `get()`
/// returns `None` and the caller should degrade gracefully.
pub struct LazyGraph {
    path: PathBuf,
    inner: OnceCell<Option<Arc<GraphStore>>>,
}

impl LazyGraph {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            inner: OnceCell::new(),
        }
    }

    /// Get or lazily initialise the `GraphStore`.
    ///
    /// Returns `None` if KuzuDB is locked or otherwise unavailable.
    pub async fn get(&self) -> Option<Arc<GraphStore>> {
        self.inner
            .get_or_init(|| async {
                match GraphStore::open(&self.path) {
                    Ok(gs) => Some(Arc::new(gs)),
                    Err(e) => {
                        warn!(
                            path = %self.path.display(),
                            error = %e,
                            "KuzuDB unavailable (locked by another process?), graph writes will be skipped"
                        );
                        None
                    }
                }
            })
            .await
            .clone()
    }
}

/// The unified storage handle. Pass this (wrapped in `Arc`) through the app.
pub struct Store {
    /// Vector similarity search over memories and entities.
    pub lance: Arc<LanceStore>,
    /// Knowledge graph of entities and relationships.
    ///
    /// For trusty-telegram this is eagerly opened; for trusty-daemon it is lazy
    /// so the daemon can start even when trusty-telegram holds the KuzuDB lock.
    pub graph: LazyGraph,
    /// Auth tokens, history cursors, and application config.
    pub sqlite: Arc<SqliteStore>,
}

impl Store {
    /// Open all three storage backends rooted at `data_dir` (eager KuzuDB init).
    ///
    /// Use this in trusty-telegram, which starts first and must own the KuzuDB lock.
    ///
    /// `user_id` is used to tag LanceDB records (single-tenant: one value per instance).
    ///
    /// Directories are created automatically if they do not exist.
    pub async fn open(data_dir: &Path, user_id: &str) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let kuzu_path = data_dir.join("kuzu");

        // Eager: fail fast if KuzuDB cannot be opened.
        let graph_store = GraphStore::open(&kuzu_path)?;
        let lazy = LazyGraph {
            path: kuzu_path,
            inner: OnceCell::new_with(Some(Some(Arc::new(graph_store)))),
        };

        Ok(Self {
            lance: Arc::new(LanceStore::open(&data_dir.join("lance"), user_id).await?),
            graph: lazy,
            sqlite: Arc::new(SqliteStore::open(&data_dir.join("trusty.db"))?),
        })
    }

    /// Count rows in the `entities` and `memories` LanceDB tables.
    ///
    /// Returns `(entity_count, memory_count)`. Both are 0 on error.
    pub async fn count_vectors(&self) -> Result<(u64, u64)> {
        self.lance.count_rows().await
    }

    /// Open storage backends rooted at `data_dir` with **lazy** KuzuDB init.
    ///
    /// Use this in trusty-daemon: SQLite and LanceDB open immediately, but
    /// KuzuDB is deferred until first access. If another process (trusty-telegram)
    /// holds the exclusive KuzuDB lock, graph operations degrade gracefully rather
    /// than preventing the daemon from starting.
    pub async fn open_lazy_kuzu(data_dir: &Path, user_id: &str) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        Ok(Self {
            lance: Arc::new(LanceStore::open(&data_dir.join("lance"), user_id).await?),
            graph: LazyGraph::new(data_dir.join("kuzu")),
            sqlite: Arc::new(SqliteStore::open(&data_dir.join("trusty.db"))?),
        })
    }
}
