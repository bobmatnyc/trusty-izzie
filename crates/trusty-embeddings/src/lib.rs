//! Local embedding generation (fastembed) and hybrid search (BM25 + RRF).
//!
//! All embedding work is done in-process using ONNX Runtime via `fastembed`.
//! No external API calls are made; the model is downloaded once and cached.

pub mod bm25;
pub mod embedder;
pub mod hybrid;

pub use embedder::{Embedder, EmbeddingModel};
pub use hybrid::{HybridSearchResult, HybridSearcher, SearchMode};
