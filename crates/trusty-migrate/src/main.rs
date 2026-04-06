//! One-time migration: add `id` column to Python-created LanceDB tables.
//!
//! Reads the existing `entities` and `memories` tables (Python schema, no id),
//! adds a stable UUID `id` derived from a SHA-256 content hash, and rewrites
//! the tables with the canonical Rust schema using `CreateTableMode::Overwrite`.
//!
//! Safe to re-run: overwrites in place (atomic at the LanceDB level).

use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{
    Array, BooleanArray, FixedSizeListArray, Float32Array, Int32Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use lancedb::database::CreateTableMode;
use lancedb::query::ExecutableQuery;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

const EMBEDDING_DIM: i32 = 384;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    trusty_core::init_logging("info");

    let data_dir = std::env::var("TRUSTY_DATA_DIR")
        .unwrap_or_else(|_| "~/.local/share/trusty-izzie".to_string())
        .replace('~', &std::env::var("HOME").unwrap_or_default());
    let lance_dir = format!("{}/lance", data_dir);

    info!("Migrating LanceDB at {}", lance_dir);

    let conn = lancedb::connect(&lance_dir)
        .execute()
        .await
        .with_context(|| format!("Failed to open LanceDB at {}", lance_dir))?;

    migrate_entities(&conn).await?;
    migrate_memories(&conn).await?;

    info!("Migration complete.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Entity migration
// ---------------------------------------------------------------------------

fn canonical_entity_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("user_id", DataType::Utf8, false),
        Field::new("entity_type", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, false),
        Field::new("normalized", DataType::Utf8, false),
        Field::new("confidence", DataType::Float32, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("source_id", DataType::Utf8, false),
        Field::new("context", DataType::Utf8, false),
        Field::new("aliases", DataType::Utf8, false),
        Field::new("occurrence_count", DataType::Int32, false),
        Field::new("first_seen", DataType::Utf8, false),
        Field::new("last_seen", DataType::Utf8, false),
        Field::new("created_at", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_DIM,
            ),
            false,
        ),
    ]))
}

async fn migrate_entities(conn: &lancedb::Connection) -> Result<()> {
    info!("Opening entities table…");
    let table = conn
        .open_table("entities")
        .execute()
        .await
        .context("Failed to open entities table")?;

    // Full scan — no filter so we get deleted rows too.
    let stream = table.query().execute().await?;
    let batches = collect_stream(stream).await?;

    let schema = canonical_entity_schema();
    let mut out_batches: Vec<RecordBatch> = Vec::new();
    let mut total = 0usize;

    for batch in &batches {
        let out = rewrite_entity_batch(batch, schema.clone())?;
        total += out.num_rows();
        out_batches.push(out);
    }

    info!("Rewriting {} entity rows with canonical schema…", total);

    conn.create_table("entities", out_batches)
        .mode(CreateTableMode::Overwrite)
        .execute()
        .await
        .context("Failed to overwrite entities table")?;

    info!("entities table migrated: {} rows", total);
    Ok(())
}

fn rewrite_entity_batch(batch: &RecordBatch, schema: SchemaRef) -> Result<RecordBatch> {
    let n = batch.num_rows();

    let get_str_col = |name: &str| -> Vec<String> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| (0..n).map(|i| a.value(i).to_string()).collect())
            .unwrap_or_else(|| vec![String::new(); n])
    };
    let get_f32_col = |name: &str| -> Vec<f32> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
            .map(|a| (0..n).map(|i| a.value(i)).collect())
            .unwrap_or_else(|| vec![0.0_f32; n])
    };
    let get_i32_col = |name: &str| -> Vec<i32> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Int32Array>())
            .map(|a| (0..n).map(|i| a.value(i)).collect())
            .unwrap_or_else(|| vec![0i32; n])
    };

    let user_ids = get_str_col("user_id");
    let entity_types = get_str_col("entity_type");
    // Support both "value" (Python schema) and "name" (old Rust schema)
    let values: Vec<String> = {
        let v = get_str_col("value");
        let fallback = get_str_col("name");
        v.into_iter()
            .zip(fallback)
            .map(|(val, fb)| if val.is_empty() { fb } else { val })
            .collect()
    };
    let normalized = get_str_col("normalized");
    let confidence = get_f32_col("confidence");
    let source = get_str_col("source");
    // Support "source_id" (Python) or derive from "source_emails" JSON (old Rust)
    let source_ids: Vec<String> = {
        let direct = get_str_col("source_id");
        let json_col = get_str_col("source_emails");
        direct
            .into_iter()
            .zip(json_col)
            .map(|(s, emails_json)| {
                if !s.is_empty() {
                    s
                } else {
                    // extract first element from JSON array
                    serde_json::from_str::<Vec<String>>(&emails_json)
                        .ok()
                        .and_then(|v| v.into_iter().next())
                        .unwrap_or_default()
                }
            })
            .collect()
    };
    let context = get_str_col("context");
    // Support "aliases" (Python) or "attributes" (old Rust)
    let aliases: Vec<String> = {
        let a = get_str_col("aliases");
        let fb = get_str_col("attributes");
        a.into_iter()
            .zip(fb)
            .map(|(alias, attr)| if alias.is_empty() { attr } else { alias })
            .collect()
    };
    // Support "occurrence_count" Int32 (Python) or "seen_count" UInt32 (old Rust)
    let occurrence_counts: Vec<i32> = {
        let from_i32 = get_i32_col("occurrence_count");
        let from_u32: Vec<i32> = batch
            .column_by_name("seen_count")
            .and_then(|c| c.as_any().downcast_ref::<arrow_array::UInt32Array>())
            .map(|a| (0..n).map(|i| a.value(i) as i32).collect())
            .unwrap_or_else(|| vec![0i32; n]);
        from_i32
            .into_iter()
            .zip(from_u32)
            .map(|(a, b)| {
                if a > 0 {
                    a
                } else if b > 0 {
                    b
                } else {
                    1
                }
            })
            .collect()
    };
    let first_seen = get_str_col("first_seen");
    let last_seen = get_str_col("last_seen");
    let created_at: Vec<String> = {
        let ca = get_str_col("created_at");
        ca.into_iter()
            .zip(first_seen.iter())
            .map(|(c, fs)| if c.is_empty() { fs.clone() } else { c })
            .collect()
    };

    // Generate canonical ids
    let ids: Vec<String> = (0..n)
        .map(|i| {
            let existing = get_str_col("id");
            let raw = &existing[i];
            if !raw.is_empty() && uuid::Uuid::parse_str(raw).is_ok() {
                raw.clone()
            } else {
                entity_id(&entity_types[i], &normalized[i], &user_ids[i])
            }
        })
        .collect();

    // Extract vector column — field may be named "vector" or "embedding"
    let vector_col = extract_vector_col(batch, n)?;

    let arrays: Vec<Arc<dyn Array>> = vec![
        Arc::new(StringArray::from(ids)),
        Arc::new(StringArray::from(user_ids)),
        Arc::new(StringArray::from(entity_types)),
        Arc::new(StringArray::from(values)),
        Arc::new(StringArray::from(normalized)),
        Arc::new(Float32Array::from(confidence)),
        Arc::new(StringArray::from(source)),
        Arc::new(StringArray::from(source_ids)),
        Arc::new(StringArray::from(context)),
        Arc::new(StringArray::from(aliases)),
        Arc::new(Int32Array::from(occurrence_counts)),
        Arc::new(StringArray::from(first_seen)),
        Arc::new(StringArray::from(last_seen)),
        Arc::new(StringArray::from(created_at)),
        vector_col,
    ];

    Ok(RecordBatch::try_new(schema, arrays)?)
}

// ---------------------------------------------------------------------------
// Memory migration
// ---------------------------------------------------------------------------

fn canonical_memory_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("user_id", DataType::Utf8, false),
        Field::new("content", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("source_type", DataType::Utf8, false),
        Field::new("source_id", DataType::Utf8, false),
        Field::new("importance", DataType::Float32, false),
        Field::new("decay_rate", DataType::Float32, false),
        Field::new("confidence", DataType::Float32, false),
        Field::new("last_accessed", DataType::Utf8, false),
        Field::new("expires_at", DataType::Utf8, false),
        Field::new("related_entities", DataType::Utf8, false),
        Field::new("tags", DataType::Utf8, false),
        Field::new("created_at", DataType::Utf8, false),
        Field::new("is_deleted", DataType::Boolean, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_DIM,
            ),
            false,
        ),
    ]))
}

async fn migrate_memories(conn: &lancedb::Connection) -> Result<()> {
    info!("Opening memories table…");
    let table = conn
        .open_table("memories")
        .execute()
        .await
        .context("Failed to open memories table")?;

    let stream = table.query().execute().await?;
    let batches = collect_stream(stream).await?;

    let schema = canonical_memory_schema();
    let mut out_batches: Vec<RecordBatch> = Vec::new();
    let mut total = 0usize;

    for batch in &batches {
        let out = rewrite_memory_batch(batch, schema.clone())?;
        total += out.num_rows();
        out_batches.push(out);
    }

    info!("Rewriting {} memory rows with canonical schema…", total);

    conn.create_table("memories", out_batches)
        .mode(CreateTableMode::Overwrite)
        .execute()
        .await
        .context("Failed to overwrite memories table")?;

    info!("memories table migrated: {} rows", total);
    Ok(())
}

fn rewrite_memory_batch(batch: &RecordBatch, schema: SchemaRef) -> Result<RecordBatch> {
    let n = batch.num_rows();

    let get_str_col = |name: &str| -> Vec<String> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| (0..n).map(|i| a.value(i).to_string()).collect())
            .unwrap_or_else(|| vec![String::new(); n])
    };
    let get_f32_col = |name: &str| -> Vec<f32> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
            .map(|a| (0..n).map(|i| a.value(i)).collect())
            .unwrap_or_else(|| vec![0.0_f32; n])
    };
    let get_bool_col = |name: &str| -> Vec<bool> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
            .map(|a| (0..n).map(|i| a.value(i)).collect())
            .unwrap_or_else(|| vec![false; n])
    };

    let user_ids = get_str_col("user_id");
    let content = get_str_col("content");
    let category = get_str_col("category");
    // source_type: Python schema has this; old Rust did not
    let source_type: Vec<String> = {
        let st = get_str_col("source_type");
        st.into_iter()
            .map(|s| if s.is_empty() { "email".to_string() } else { s })
            .collect()
    };
    let source_id = get_str_col("source_id");
    let importance = get_f32_col("importance");
    let decay_rate = get_f32_col("decay_rate");
    let confidence = get_f32_col("confidence");
    let last_accessed = get_str_col("last_accessed");
    let expires_at = get_str_col("expires_at");
    // related_entities: Python has this; old Rust used "tags" for this
    let related_entities: Vec<String> = {
        let re = get_str_col("related_entities");
        let fb = get_str_col("tags");
        re.into_iter()
            .zip(fb)
            .map(|(r, t)| if r.is_empty() { t } else { r })
            .collect()
    };
    // tags: keep as separate field (empty JSON array if missing)
    let tags: Vec<String> = {
        let t = get_str_col("tags");
        t.into_iter()
            .map(|s| if s.is_empty() { "[]".to_string() } else { s })
            .collect()
    };
    let created_at = get_str_col("created_at");
    // is_deleted: canonical. Legacy: "archived"
    let is_deleted: Vec<bool> = {
        let d = get_bool_col("is_deleted");
        let a = get_bool_col("archived");
        d.into_iter().zip(a).map(|(d, a)| d || a).collect()
    };

    // Generate canonical ids
    let ids: Vec<String> = {
        let existing = get_str_col("id");
        existing
            .into_iter()
            .zip(content.iter())
            .zip(category.iter())
            .map(|((raw, c), cat)| {
                if !raw.is_empty() && uuid::Uuid::parse_str(&raw).is_ok() {
                    raw
                } else {
                    memory_id(c, cat)
                }
            })
            .collect()
    };

    let vector_col = extract_vector_col(batch, n)?;

    // Clamp confidence: if all zeros (not present in Python schema), set to 1.0
    let confidence: Vec<f32> = confidence
        .into_iter()
        .map(|c| if c == 0.0 { 1.0 } else { c })
        .collect();

    // If decay_rate is all zeros (not in Python schema), compute from category
    let decay_rate: Vec<f32> = decay_rate
        .into_iter()
        .zip(category.iter())
        .map(|(dr, cat)| {
            if dr == 0.0 {
                category_decay_rate(cat)
            } else {
                dr
            }
        })
        .collect();

    let arrays: Vec<Arc<dyn Array>> = vec![
        Arc::new(StringArray::from(ids)),
        Arc::new(StringArray::from(user_ids)),
        Arc::new(StringArray::from(content)),
        Arc::new(StringArray::from(category)),
        Arc::new(StringArray::from(source_type)),
        Arc::new(StringArray::from(source_id)),
        Arc::new(Float32Array::from(importance)),
        Arc::new(Float32Array::from(decay_rate)),
        Arc::new(Float32Array::from(confidence)),
        Arc::new(StringArray::from(last_accessed)),
        Arc::new(StringArray::from(expires_at)),
        Arc::new(StringArray::from(related_entities)),
        Arc::new(StringArray::from(tags)),
        Arc::new(StringArray::from(created_at)),
        Arc::new(BooleanArray::from(is_deleted)),
        vector_col,
    ];

    Ok(RecordBatch::try_new(schema, arrays)?)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Extract the vector column, accepting both "vector" and "embedding" field names.
fn extract_vector_col(batch: &RecordBatch, _n: usize) -> Result<Arc<dyn Array>> {
    let col = batch
        .column_by_name("vector")
        .or_else(|| batch.column_by_name("embedding"))
        .ok_or_else(|| anyhow::anyhow!("no vector/embedding column found in batch"))?;

    // Validate shape: must be FixedSizeList of 384 floats
    match col.data_type() {
        DataType::FixedSizeList(item_field, size) => {
            if *size != EMBEDDING_DIM {
                warn!(
                    "Unexpected embedding dimension {} (expected {}), using as-is",
                    size, EMBEDDING_DIM
                );
            }
            // Rebuild as canonical "vector" field name
            let item_f = item_field.clone();
            let values_array = col
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .ok_or_else(|| anyhow::anyhow!("could not downcast to FixedSizeListArray"))?
                .values()
                .clone();
            let new_arr = FixedSizeListArray::new(item_f, EMBEDDING_DIM, values_array, None);
            Ok(Arc::new(new_arr))
        }
        other => anyhow::bail!("unexpected vector column type: {:?}", other),
    }
}

async fn collect_stream(
    stream: lancedb::arrow::SendableRecordBatchStream,
) -> Result<Vec<RecordBatch>> {
    use futures::Stream;
    use std::task::Poll;

    let mut batches = Vec::new();
    futures::pin_mut!(stream);
    loop {
        let item = futures::future::poll_fn(|cx| match stream.as_mut().poll_next(cx) {
            Poll::Ready(v) => Poll::Ready(v),
            Poll::Pending => Poll::Pending,
        })
        .await;
        match item {
            None => break,
            Some(Ok(b)) => batches.push(b),
            Some(Err(e)) => return Err(anyhow::anyhow!("stream error: {}", e)),
        }
    }
    Ok(batches)
}

fn entity_id(entity_type: &str, normalized: &str, user_id: &str) -> String {
    let key = format!("{}:{}:{}", entity_type, normalized, user_id);
    let hash = Sha256::digest(key.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn memory_id(content: &str, category: &str) -> String {
    let n = content.len().min(128);
    let key = format!("{}:{}", category, &content[..n]);
    let hash = Sha256::digest(key.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn category_decay_rate(category: &str) -> f32 {
    let half_life_days: f32 = match category {
        "user_preference" => 365.0,
        "person_fact" => 180.0,
        "project_fact" => 90.0,
        "company_fact" => 180.0,
        "recurring_event" => 30.0,
        "decision" => 120.0,
        "event" => 60.0,
        _ => 90.0,
    };
    1.0 / (half_life_days * 86400.0)
}
