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

        let index = Index::open_or_create(
            tantivy::directory::MmapDirectory::open(index_path)?,
            schema,
        )?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
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
