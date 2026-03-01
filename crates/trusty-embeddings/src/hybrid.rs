//! Hybrid search combining dense vector similarity with BM25 via Reciprocal Rank Fusion.

use std::collections::HashMap;

use anyhow::Result;

/// Controls which retrieval signals to use in a hybrid search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchMode {
    /// Use dense vector similarity only.
    Vector,
    /// Use BM25 keyword search only.
    Keyword,
    /// Fuse both signals with Reciprocal Rank Fusion.
    Hybrid,
}

/// A single result from a hybrid search query.
#[derive(Debug, Clone)]
pub struct HybridSearchResult {
    /// The document ID.
    pub id: String,
    /// The RRF-fused relevance score (higher is better).
    pub score: f64,
    /// The original text content of the document.
    pub content: String,
    /// The dense vector rank (lower is better; `None` if not retrieved by vector).
    pub vector_rank: Option<usize>,
    /// The BM25 rank (lower is better; `None` if not retrieved by keyword).
    pub keyword_rank: Option<usize>,
}

/// Orchestrates hybrid retrieval using a vector store and a BM25 index.
///
/// Construct once, share via `Arc`.
pub struct HybridSearcher {
    /// RRF smoothing constant — 60 is the standard value from the original paper.
    pub rrf_k: usize,
    /// Weight on the vector signal; `1 - alpha` is applied to BM25.
    pub alpha: f64,
}

impl Default for HybridSearcher {
    fn default() -> Self {
        Self {
            rrf_k: 60,
            alpha: 0.7,
        }
    }
}

impl HybridSearcher {
    /// Create a new `HybridSearcher` with custom RRF parameters.
    ///
    /// * `rrf_k` — smoothing constant (60 is standard).
    /// * `alpha` — weight on the vector signal in `[0.0, 1.0]`.
    pub fn new(rrf_k: usize, alpha: f64) -> Self {
        Self { rrf_k, alpha }
    }

    /// Fuse two ranked lists of document IDs using Reciprocal Rank Fusion.
    ///
    /// Each list contributes a per-document score of `1 / (k + rank)`.
    /// The scores from both lists are summed with `alpha` applied to the
    /// vector list and `(1 - alpha)` applied to the keyword list.
    ///
    /// Returns a `Vec<(id, fused_score)>` sorted descending by score.
    pub fn rrf_fuse(&self, vector_ids: &[String], keyword_ids: &[String]) -> Vec<(String, f64)> {
        let mut scores: HashMap<String, f64> = HashMap::new();

        for (rank, id) in vector_ids.iter().enumerate() {
            let contribution = self.alpha / (self.rrf_k + rank + 1) as f64;
            *scores.entry(id.clone()).or_default() += contribution;
        }

        for (rank, id) in keyword_ids.iter().enumerate() {
            let contribution = (1.0 - self.alpha) / (self.rrf_k + rank + 1) as f64;
            *scores.entry(id.clone()).or_default() += contribution;
        }

        let mut ranked: Vec<(String, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }

    /// Run a hybrid search and return at most `limit` fused results.
    ///
    /// The caller is responsible for providing pre-ranked lists from each
    /// retrieval system. This method only handles score fusion.
    ///
    /// * `vector_results` — `(id, content)` pairs in vector-rank order.
    /// * `keyword_results` — `(id, content)` pairs in BM25-rank order.
    pub fn search(
        &self,
        vector_results: Vec<(String, String)>,
        keyword_results: Vec<(String, String)>,
        limit: usize,
        mode: SearchMode,
    ) -> Result<Vec<HybridSearchResult>> {
        // Apply mode branching before building rank structures.
        let (effective_vector, effective_keyword) = match mode {
            SearchMode::Vector => (vector_results, vec![]),
            SearchMode::Keyword => (vec![], keyword_results),
            SearchMode::Hybrid => (vector_results, keyword_results),
        };

        let vector_ids: Vec<String> = effective_vector.iter().map(|(id, _)| id.clone()).collect();
        let keyword_ids: Vec<String> = effective_keyword.iter().map(|(id, _)| id.clone()).collect();

        // Build a content lookup map (id -> content)
        let mut content_map: HashMap<String, String> = HashMap::new();
        for (id, content) in effective_vector.iter().chain(effective_keyword.iter()) {
            content_map
                .entry(id.clone())
                .or_insert_with(|| content.clone());
        }

        // Build rank lookup maps
        let vector_rank_map: HashMap<String, usize> = vector_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        let keyword_rank_map: HashMap<String, usize> = keyword_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();

        let fused = self.rrf_fuse(&vector_ids, &keyword_ids);

        let results = fused
            .into_iter()
            .take(limit)
            .map(|(id, score)| HybridSearchResult {
                content: content_map.get(&id).cloned().unwrap_or_default(),
                vector_rank: vector_rank_map.get(&id).copied(),
                keyword_rank: keyword_rank_map.get(&id).copied(),
                id,
                score,
            })
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pairs(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter()
            .map(|id| (id.to_string(), format!("content of {id}")))
            .collect()
    }

    #[test]
    fn rrf_fuse_ranks_shared_document_higher() {
        let searcher = HybridSearcher::default();
        let vector_ids = vec!["doc_a".to_string(), "doc_b".to_string()];
        let keyword_ids = vec!["doc_b".to_string(), "doc_c".to_string()];

        let fused = searcher.rrf_fuse(&vector_ids, &keyword_ids);

        // doc_b appears in both lists, should score highest
        assert_eq!(fused[0].0, "doc_b");
    }

    #[test]
    fn test_vector_only_mode() {
        let searcher = HybridSearcher::default();
        let vector = make_pairs(&["v1", "v2"]);
        let keyword = make_pairs(&["k1", "k2"]);

        let results = searcher
            .search(vector, keyword, 10, SearchMode::Vector)
            .unwrap();

        assert_eq!(results.len(), 2);
        // Vector-only: results come from vector list in vector rank order
        assert_eq!(results[0].id, "v1");
        assert_eq!(results[1].id, "v2");
        // No keyword rank assigned
        assert!(results[0].keyword_rank.is_none());
    }

    #[test]
    fn test_keyword_only_mode() {
        let searcher = HybridSearcher::default();
        let vector = make_pairs(&["v1", "v2"]);
        let keyword = make_pairs(&["k1", "k2"]);

        let results = searcher
            .search(vector, keyword, 10, SearchMode::Keyword)
            .unwrap();

        assert_eq!(results.len(), 2);
        // Keyword-only: results come from keyword list in keyword rank order
        assert_eq!(results[0].id, "k1");
        assert_eq!(results[1].id, "k2");
        // No vector rank assigned
        assert!(results[0].vector_rank.is_none());
    }

    #[test]
    fn test_limit_respected() {
        let searcher = HybridSearcher::default();
        let vector = make_pairs(&["a", "b", "c", "d", "e"]);
        let keyword = make_pairs(&["a", "b", "c", "d", "e"]);

        let results = searcher
            .search(vector, keyword, 3, SearchMode::Hybrid)
            .unwrap();

        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_rrf_scores_are_positive() {
        let searcher = HybridSearcher::default();
        let vector = make_pairs(&["x", "y"]);
        let keyword = make_pairs(&["y", "z"]);

        let results = searcher
            .search(vector, keyword, 10, SearchMode::Hybrid)
            .unwrap();

        for r in &results {
            assert!(r.score > 0.0, "expected positive score, got {}", r.score);
        }
    }

    #[test]
    fn test_empty_lists_return_empty() {
        let searcher = HybridSearcher::default();

        let results = searcher
            .search(vec![], vec![], 10, SearchMode::Hybrid)
            .unwrap();

        assert!(results.is_empty());
    }
}
