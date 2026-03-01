//! fastembed wrapper for generating dense embedding vectors locally.

use anyhow::Result;
use fastembed::{EmbeddingModel as FastembedModel, InitOptions, TextEmbedding};
use once_cell::sync::OnceCell;

/// Supported local embedding model identifiers.
#[derive(Debug, Clone)]
pub enum EmbeddingModel {
    /// `all-MiniLM-L6-v2` — 384-dimensional, fast and accurate.
    AllMiniLmL6V2,
    /// `BAAI/bge-small-en-v1.5` — 384-dimensional, strong retrieval model.
    BgeSmallEnV1_5,
}

impl EmbeddingModel {
    /// Returns the fastembed `EmbeddingModel` variant for this model.
    fn to_fastembed_model(&self) -> FastembedModel {
        match self {
            EmbeddingModel::AllMiniLmL6V2 => FastembedModel::AllMiniLML6V2,
            EmbeddingModel::BgeSmallEnV1_5 => FastembedModel::BGESmallENV15,
        }
    }

    /// Returns the output dimensionality for this model.
    pub fn dimensions(&self) -> usize {
        match self {
            EmbeddingModel::AllMiniLmL6V2 => 384,
            EmbeddingModel::BgeSmallEnV1_5 => 384,
        }
    }
}

/// Wraps a fastembed `TextEmbedding` instance with a simple API.
///
/// Construct once and share across the application via `Arc`.
pub struct Embedder {
    model: TextEmbedding,
    /// The model that was loaded.
    pub model_type: EmbeddingModel,
}

impl Embedder {
    /// Initialise the embedder, downloading the ONNX model on first run.
    ///
    /// Subsequent runs load the cached model from disk.
    pub fn new(model: EmbeddingModel) -> Result<Self> {
        let text_embedding = TextEmbedding::try_new(InitOptions::new(model.to_fastembed_model()))?;
        Ok(Self {
            model: text_embedding,
            model_type: model,
        })
    }

    /// Embed a single text string, returning a dense vector.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut results = self.model.embed(vec![text.to_owned()], None)?;
        Ok(results.remove(0))
    }

    /// Embed multiple texts in a single batch call for efficiency.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        self.model.embed(owned, None)
    }

    /// Output dimensionality of this embedder's model.
    pub fn dimensions(&self) -> usize {
        self.model_type.dimensions()
    }
}

/// A process-global embedder instance, initialised lazily.
///
/// Use this when you need a shared embedder without threading `Arc<Embedder>`
/// through every call site. Prefer explicit injection for testability.
static GLOBAL_EMBEDDER: OnceCell<Embedder> = OnceCell::new();

/// Initialise the global embedder. Returns an error if called more than once.
pub fn init_global_embedder(model: EmbeddingModel) -> Result<()> {
    let embedder = Embedder::new(model)?;
    GLOBAL_EMBEDDER
        .set(embedder)
        .map_err(|_| anyhow::anyhow!("global embedder already initialised"))?;
    Ok(())
}

/// Access the global embedder after initialisation.
pub fn global_embedder() -> Option<&'static Embedder> {
    GLOBAL_EMBEDDER.get()
}

// Run with: cargo test -p trusty-embeddings -- --ignored
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_embed_returns_correct_dimensions() {
        let embedder = Embedder::new(EmbeddingModel::AllMiniLmL6V2).unwrap();
        let vec = embedder.embed("hello world").unwrap();
        assert_eq!(vec.len(), 384);
    }

    #[test]
    #[ignore]
    fn test_embed_batch_consistency() {
        let embedder = Embedder::new(EmbeddingModel::AllMiniLmL6V2).unwrap();
        let first = embedder.embed("consistent input").unwrap();
        let second = embedder.embed("consistent input").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    #[ignore]
    fn test_embed_batch_length() {
        let embedder = Embedder::new(EmbeddingModel::AllMiniLmL6V2).unwrap();
        let texts = &["first", "second", "third"];
        let results = embedder.embed_batch(texts).unwrap();
        assert_eq!(results.len(), 3);
    }
}
