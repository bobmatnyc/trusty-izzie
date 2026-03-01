//! Tantivy-backed BM25 full-text index for sparse retrieval.

use anyhow::Result;
use std::path::Path;
use tantivy::{
    collector::TopDocs,
    doc,
    query::QueryParser,
    schema::{OwnedValue, Schema, STORED, TEXT},
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument,
};

/// A single BM25 search result.
#[derive(Debug, Clone)]
pub struct Bm25Result {
    /// The document ID that was indexed.
    pub id: String,
    /// BM25 relevance score (unnormalised).
    pub score: f32,
    /// The text content stored alongside the document.
    pub content: String,
}

/// Wraps a Tantivy index for simple BM25 retrieval.
pub struct Bm25Index {
    index: Index,
    reader: IndexReader,
    writer: std::sync::Mutex<IndexWriter>,
    id_field: tantivy::schema::Field,
    content_field: tantivy::schema::Field,
}

impl Bm25Index {
    /// Open (or create) a Tantivy index at `index_path`.
    pub fn open(index_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_path)?;

        let mut schema_builder = Schema::builder();
        let id_field = schema_builder.add_text_field("id", TEXT | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let schema = schema_builder.build();

        let index =
            Index::open_or_create(tantivy::directory::MmapDirectory::open(index_path)?, schema)?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        let writer = index.writer(50_000_000)?; // 50 MB heap

        Ok(Self {
            index,
            reader,
            writer: std::sync::Mutex::new(writer),
            id_field,
            content_field,
        })
    }

    /// Index a document by `id` with the given `content`.
    ///
    /// Replaces any existing document with the same `id` by delete-then-add.
    pub fn index_document(&self, id: &str, content: &str) -> Result<()> {
        let mut writer = self.writer.lock().unwrap();

        // Delete existing document with this id
        let id_term = tantivy::Term::from_field_text(self.id_field, id);
        writer.delete_term(id_term);

        writer.add_document(doc!(
            self.id_field => id,
            self.content_field => content,
        ))?;

        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Search for documents matching `query`, returning at most `limit` results.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Bm25Result>> {
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.content_field]);
        let parsed = query_parser.parse_query(query)?;

        let top_docs = searcher.search(&parsed, &TopDocs::with_limit(limit))?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;

            let id = doc
                .get_first(self.id_field)
                .and_then(|v| match v {
                    OwnedValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("")
                .to_string();

            let content = doc
                .get_first(self.content_field)
                .and_then(|v| match v {
                    OwnedValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("")
                .to_string();

            results.push(Bm25Result { id, score, content });
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_index_and_search_basic() {
        let dir = TempDir::new().unwrap();
        let index = Bm25Index::open(dir.path()).unwrap();

        index.index_document("doc1", "the quick brown fox").unwrap();
        index.index_document("doc2", "lazy dog sleeps").unwrap();
        index
            .index_document("doc3", "rust programming language")
            .unwrap();

        let results = index.search("fox", 10).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "doc1");
    }

    #[test]
    fn test_update_document() {
        let dir = TempDir::new().unwrap();
        let index = Bm25Index::open(dir.path()).unwrap();

        index
            .index_document("doc1", "original content here")
            .unwrap();
        index
            .index_document("doc1", "updated content replacement")
            .unwrap();

        // "original" should no longer match after update
        let old_results = index.search("original", 10).unwrap();
        assert!(old_results.is_empty());

        // "replacement" should match the updated content
        let new_results = index.search("replacement", 10).unwrap();
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].id, "doc1");
        assert!(new_results[0].content.contains("replacement"));
    }

    #[test]
    fn test_search_empty_index() {
        let dir = TempDir::new().unwrap();
        let index = Bm25Index::open(dir.path()).unwrap();

        let results = index.search("anything", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_no_match() {
        let dir = TempDir::new().unwrap();
        let index = Bm25Index::open(dir.path()).unwrap();

        index.index_document("doc1", "the quick brown fox").unwrap();
        index.index_document("doc2", "lazy dog sleeps").unwrap();

        let results = index.search("elephant", 10).unwrap();
        assert!(results.is_empty());
    }
}
