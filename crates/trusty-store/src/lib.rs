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
    read_only: bool,
    inner: OnceCell<Option<Arc<GraphStore>>>,
}

impl LazyGraph {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            read_only: false,
            inner: OnceCell::new(),
        }
    }

    pub fn new_read_only(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            read_only: true,
            inner: OnceCell::new(),
        }
    }

    /// Get or lazily initialise the `GraphStore`.
    ///
    /// Returns `None` if KuzuDB is locked or otherwise unavailable.
    pub async fn get(&self) -> Option<Arc<GraphStore>> {
        self.inner
            .get_or_init(|| async {
                let result = if self.read_only {
                    GraphStore::open_read_only(&self.path)
                } else {
                    GraphStore::open(&self.path)
                };
                match result {
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
    /// Only trusty-daemon opens this read-write. All other processes
    /// (trusty-api, trusty-telegram, trusty-mcp) open read-only to avoid
    /// Kuzu lock contention.
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
        Self::open_with_mode(data_dir, user_id, false).await
    }

    /// Open all three storage backends with KuzuDB in read-only mode.
    ///
    /// Use this in non-daemon processes (trusty-api, trusty-telegram, trusty-mcp)
    /// to avoid Kuzu lock contention. Only the daemon should open read-write.
    pub async fn open_read_only(data_dir: &Path, user_id: &str) -> Result<Self> {
        Self::open_with_mode(data_dir, user_id, true).await
    }

    /// Open all three storage backends with the specified KuzuDB access mode.
    async fn open_with_mode(data_dir: &Path, user_id: &str, read_only: bool) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let kuzu_path = data_dir.join("kuzu");

        // Eager: fail fast if KuzuDB cannot be opened.
        let graph_store = GraphStore::open_with_mode(&kuzu_path, read_only)?;
        let lazy = LazyGraph {
            path: kuzu_path,
            read_only,
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

    /// Open storage backends with **lazy** KuzuDB init in **read-only** mode.
    ///
    /// Use this in non-daemon processes (trusty-api, trusty-mcp) that need lazy
    /// init but should not contend for the Kuzu write lock.
    pub async fn open_lazy_kuzu_read_only(data_dir: &Path, user_id: &str) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        Ok(Self {
            lance: Arc::new(LanceStore::open(&data_dir.join("lance"), user_id).await?),
            graph: LazyGraph::new_read_only(data_dir.join("kuzu")),
            sqlite: Arc::new(SqliteStore::open(&data_dir.join("trusty.db"))?),
        })
    }
}
