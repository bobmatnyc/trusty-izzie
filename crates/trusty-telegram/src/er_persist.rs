//! Persist entity extraction results to the store layer.

use std::sync::Arc;

use anyhow::Result;
use sha2::{Digest, Sha256};
use tracing::warn;

use trusty_extractor::ExtractionResult;
use trusty_store::Store;

/// Statistics from a single persist pass.
pub struct PersistStats {
    pub entities_written: usize,
    pub entities_staged: usize,
    pub relationships_written: usize,
}

/// Persist an `ExtractionResult` to LanceDB, Kuzu, and SQLite.
///
/// Entities are gated behind `min_occurrences`: only written to the vector
/// and graph stores once they've been seen at least that many times.
/// Relationships are written unconditionally (Kuzu silently skips if nodes
/// are missing).
pub async fn persist_extraction_result(
    result: &ExtractionResult,
    store: &Arc<Store>,
    min_occurrences: u32,
) -> Result<PersistStats> {
    let mut entities_written = 0usize;
    let mut entities_staged = 0usize;
    let mut relationships_written = 0usize;

    for entity in &result.entities {
        // Compute fingerprint: SHA256("{entity_type}:{normalized}")[:16] as hex.
        let key = format!(
            "{}:{}",
            format!("{:?}", entity.entity_type).to_lowercase(),
            entity.normalized
        );
        let hash = Sha256::digest(key.as_bytes());
        let fp: String = hex::encode(&hash[..8]); // 8 bytes = 16 hex chars

        let entity_type_str = format!("{:?}", entity.entity_type);
        let entity_id_str = entity.id.to_string();

        // Upsert fingerprint (increments seen_count atomically).
        store.sqlite.upsert_fingerprint(
            &fp,
            &entity_id_str,
            &entity_type_str,
            &entity.normalized,
        )?;

        // Read back the current count.
        let seen_count = store.sqlite.get_fingerprint_count(&fp)?;

        if seen_count >= min_occurrences {
            // Write to LanceDB (zero vector until embeddings are implemented).
            if let Err(e) = store.lance.upsert_entity(entity, vec![0.0f32; 384]).await {
                warn!(entity = %entity.normalized, error = %e, "failed to upsert entity to LanceDB");
            }

            // Write to Kuzu (sync call — must use spawn_blocking).
            let graph = Arc::clone(&store.graph);
            let entity_clone = entity.clone();
            if let Err(e) =
                tokio::task::spawn_blocking(move || graph.upsert_entity(&entity_clone)).await
            {
                warn!(entity = %entity.normalized, error = %e, "failed to upsert entity to Kuzu");
            }

            // Mark as graduated so we don't double-write on next pass.
            let _ = store.sqlite.mark_fingerprint_graduated(&fp);

            entities_written += 1;
        } else {
            entities_staged += 1;
        }
    }

    for rel in &result.relationships {
        let graph = Arc::clone(&store.graph);
        let rel_clone = rel.clone();
        if let Err(e) =
            tokio::task::spawn_blocking(move || graph.upsert_relationship(&rel_clone)).await
        {
            warn!(error = %e, "failed to upsert relationship to Kuzu");
        } else {
            relationships_written += 1;
        }
    }

    Ok(PersistStats {
        entities_written,
        entities_staged,
        relationships_written,
    })
}
