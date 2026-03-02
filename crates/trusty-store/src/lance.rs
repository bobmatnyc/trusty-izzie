//! LanceDB vector store for memory and entity embeddings.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use arrow_array::{
    Array, BooleanArray, FixedSizeListArray, Float32Array, Int32Array, RecordBatch,
    RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::Stream;
use lancedb::query::{ExecutableQuery, QueryBase};
use sha2::{Digest, Sha256};
use tracing::debug;

use trusty_models::entity::{Entity, EntityType};
use trusty_models::memory::{Memory, MemoryCategory};

const EMBEDDING_DIM: i32 = 384;

/// Collect a `SendableRecordBatchStream` into a `Vec<RecordBatch>`.
async fn collect_stream(
    stream: lancedb::arrow::SendableRecordBatchStream,
) -> Result<Vec<RecordBatch>> {
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
            Some(Ok(batch)) => batches.push(batch),
            Some(Err(e)) => return Err(anyhow!("stream error: {}", e)),
        }
    }
    Ok(batches)
}

/// Canonical entity schema — matches Python migration output plus `id` column.
///
/// Field order must exactly match the column array order in `upsert_entity`.
fn entity_schema() -> Arc<Schema> {
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

/// Canonical memory schema — matches Python migration output plus `id` column.
///
/// Field order must exactly match the column array order in `upsert_memory`.
fn memory_schema() -> Arc<Schema> {
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

/// Build a FixedSizeList column from a flat Vec<f32>.
fn make_embedding_column(embedding: Vec<f32>, schema: &Arc<Schema>) -> Result<Arc<dyn Array>> {
    let emb_field = schema
        .field_with_name("vector")
        .map_err(|e| anyhow!("schema missing vector: {}", e))?;
    let item_field = match emb_field.data_type() {
        DataType::FixedSizeList(f, _) => f.clone(),
        _ => return Err(anyhow!("unexpected vector type")),
    };

    let values = Arc::new(Float32Array::from(embedding));
    let arr = FixedSizeListArray::new(item_field, EMBEDDING_DIM, values, None);
    Ok(Arc::new(arr))
}

/// Open a table or create it with the given schema (safe for existing live data).
async fn open_or_create(
    conn: &lancedb::Connection,
    name: &str,
    schema: Arc<Schema>,
) -> Result<lancedb::Table> {
    match conn.open_table(name).execute().await {
        Ok(tbl) => Ok(tbl),
        Err(_) => {
            // Table doesn't exist — create it empty
            let empty_batch = RecordBatch::new_empty(schema.clone());
            let batches = RecordBatchIterator::new(vec![Ok(empty_batch)], schema);
            let tbl = conn.create_table(name, batches).execute().await?;
            Ok(tbl)
        }
    }
}

/// Handle to the LanceDB vector store.
pub struct LanceStore {
    connection: lancedb::Connection,
    pub user_id: String,
}

impl LanceStore {
    /// Open (or create) a LanceDB database at `path`.
    pub async fn open(path: &Path, user_id: &str) -> Result<Self> {
        let uri = path.to_string_lossy();
        let connection = lancedb::connect(&uri).execute().await?;

        // Ensure both tables exist (idempotent).
        open_or_create(&connection, "entities", entity_schema()).await?;
        open_or_create(&connection, "memories", memory_schema()).await?;

        Ok(Self {
            connection,
            user_id: user_id.to_string(),
        })
    }

    /// Upsert an entity by ID (delete-then-add).
    pub async fn upsert_entity(&self, entity: &Entity, embedding: Vec<f32>) -> Result<()> {
        let table = self.connection.open_table("entities").execute().await?;

        let id_str = entity.id.to_string();
        // Delete existing row if present
        table.delete(&format!("id = '{}'", id_str)).await?;

        let schema = entity_schema();
        let aliases_json = serde_json::to_string(&entity.aliases)?;
        let source_id = entity.source_id.as_deref().unwrap_or("").to_string();
        let context = entity.context.as_deref().unwrap_or("").to_string();
        let created_at = entity.first_seen.to_rfc3339();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id_str.as_str()])),
                Arc::new(StringArray::from(vec![entity.user_id.as_str()])),
                Arc::new(StringArray::from(vec![entity_type_str(
                    &entity.entity_type,
                )])),
                Arc::new(StringArray::from(vec![entity.value.as_str()])),
                Arc::new(StringArray::from(vec![entity.normalized.as_str()])),
                Arc::new(Float32Array::from(vec![entity.confidence])),
                Arc::new(StringArray::from(vec![entity.source.as_str()])),
                Arc::new(StringArray::from(vec![source_id.as_str()])),
                Arc::new(StringArray::from(vec![context.as_str()])),
                Arc::new(StringArray::from(vec![aliases_json.as_str()])),
                Arc::new(Int32Array::from(vec![entity.occurrence_count as i32])),
                Arc::new(StringArray::from(vec![entity
                    .first_seen
                    .to_rfc3339()
                    .as_str()])),
                Arc::new(StringArray::from(vec![entity
                    .last_seen
                    .to_rfc3339()
                    .as_str()])),
                Arc::new(StringArray::from(vec![created_at.as_str()])),
                make_embedding_column(embedding, &schema)?,
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(batches).execute().await?;
        debug!("upserted entity {}", entity.id);
        Ok(())
    }

    /// Vector similarity search over entities. Returns (entity_id, distance) pairs.
    pub async fn search_entities(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        let table = self.connection.open_table("entities").execute().await?;

        let query_vec: Vec<f32> = query_embedding.to_vec();
        let stream = table
            .query()
            .nearest_to(query_vec)?
            .limit(limit)
            .execute()
            .await?;

        let batches: Vec<RecordBatch> = collect_stream(stream).await?;
        let mut results = Vec::new();
        for batch in &batches {
            let id_col = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| anyhow!("missing id column in search result"))?;
            // LanceDB adds a "_distance" column for vector searches
            let dist_col = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>());

            for i in 0..batch.num_rows() {
                let id = id_col.value(i).to_string();
                let dist = dist_col.map(|c| c.value(i)).unwrap_or(0.0);
                results.push((id, dist));
            }
        }
        Ok(results)
    }

    /// Get an entity by its UUID string. Returns None if not found.
    pub async fn get_entity_by_id(&self, id: &str) -> Result<Option<Entity>> {
        let table = self.connection.open_table("entities").execute().await?;

        let filter = format!("id = '{}'", id);
        let stream = table.query().only_if(filter).limit(1).execute().await?;

        let batches: Vec<RecordBatch> = collect_stream(stream).await?;
        for batch in &batches {
            if batch.num_rows() > 0 {
                if let Some(entity) = entity_from_batch(batch, 0)? {
                    return Ok(Some(entity));
                }
            }
        }
        Ok(None)
    }

    /// Upsert a memory by ID (delete-then-add).
    pub async fn upsert_memory(&self, memory: &Memory, embedding: Vec<f32>) -> Result<()> {
        let table = self.connection.open_table("memories").execute().await?;

        let id_str = memory.id.to_string();
        table.delete(&format!("id = '{}'", id_str)).await?;

        let schema = memory_schema();
        let related_entities_json = serde_json::to_string(&memory.related_entities)?;
        let tags_json = serde_json::json!([]).to_string();
        let last_accessed = memory
            .last_accessed
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        let decay_rate = 1.0_f32 / (memory.category.decay_half_life_days() * 86400.0);
        let source_id = memory.source_id.as_deref().unwrap_or("").to_string();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id_str.as_str()])),
                Arc::new(StringArray::from(vec![memory.user_id.as_str()])),
                Arc::new(StringArray::from(vec![memory.content.as_str()])),
                Arc::new(StringArray::from(vec![memory_category_str(
                    &memory.category,
                )])),
                Arc::new(StringArray::from(vec!["email"])),
                Arc::new(StringArray::from(vec![source_id.as_str()])),
                Arc::new(Float32Array::from(vec![memory.importance])),
                Arc::new(Float32Array::from(vec![decay_rate])),
                Arc::new(Float32Array::from(vec![1.0_f32])), // confidence
                Arc::new(StringArray::from(vec![last_accessed.as_str()])),
                Arc::new(StringArray::from(vec![""])), // expires_at: empty = no expiry
                Arc::new(StringArray::from(vec![related_entities_json.as_str()])),
                Arc::new(StringArray::from(vec![tags_json.as_str()])),
                Arc::new(StringArray::from(vec![memory
                    .created_at
                    .to_rfc3339()
                    .as_str()])),
                Arc::new(BooleanArray::from(vec![false])), // is_deleted
                make_embedding_column(embedding, &schema)?,
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(batches).execute().await?;
        debug!("upserted memory {}", memory.id);
        Ok(())
    }

    /// Vector similarity search over memories. Returns (memory_id, distance) pairs.
    pub async fn search_memories(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        let table = self.connection.open_table("memories").execute().await?;

        let query_vec: Vec<f32> = query_embedding.to_vec();
        let stream = table
            .query()
            .nearest_to(query_vec)?
            .limit(limit)
            .execute()
            .await?;

        let batches: Vec<RecordBatch> = collect_stream(stream).await?;
        let mut results = Vec::new();
        for batch in &batches {
            let id_col = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| anyhow!("missing id column in memory search result"))?;
            let dist_col = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>());

            for i in 0..batch.num_rows() {
                let id = id_col.value(i).to_string();
                let dist = dist_col.map(|c| c.value(i)).unwrap_or(0.0);
                results.push((id, dist));
            }
        }
        Ok(results)
    }

    /// Get a memory by its UUID string.
    pub async fn get_memory_by_id(&self, id: &str) -> Result<Option<Memory>> {
        let table = self.connection.open_table("memories").execute().await?;

        let filter = format!("id = '{}'", id);
        let stream = table.query().only_if(filter).limit(1).execute().await?;

        let batches: Vec<RecordBatch> = collect_stream(stream).await?;
        for batch in &batches {
            if batch.num_rows() > 0 {
                if let Some(memory) = memory_from_batch(batch, 0)? {
                    return Ok(Some(memory));
                }
            }
        }
        Ok(None)
    }

    /// List memories by scanning the table (no ANN), sorted by importance desc.
    /// Returns up to `limit` Memory records.
    pub async fn list_memories(&self, limit: usize) -> Result<Vec<Memory>> {
        let table = self.connection.open_table("memories").execute().await?;

        let stream = table
            .query()
            .only_if("is_deleted = false")
            .limit(limit)
            .execute()
            .await?;

        let batches: Vec<RecordBatch> = collect_stream(stream).await?;
        let mut memories = Vec::new();
        for batch in &batches {
            for row in 0..batch.num_rows() {
                if let Ok(Some(m)) = memory_from_batch(batch, row) {
                    memories.push(m);
                }
            }
        }
        memories.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        memories.truncate(limit);
        Ok(memories)
    }

    /// List all entities, optionally filtered by type string (e.g. "Person").
    ///
    /// Results are sorted by `confidence` descending. Uses a full table scan —
    /// suitable for CLI browsing against the 6,774-entity migrated data set.
    pub async fn list_entities(
        &self,
        entity_type_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Entity>> {
        let table = self.connection.open_table("entities").execute().await?;

        let mut q = table.query();
        if let Some(et) = entity_type_filter {
            q = q.only_if(format!("entity_type = '{}' ", et));
        }
        let stream = q.limit(limit * 4).execute().await?; // over-fetch for sort

        let batches: Vec<RecordBatch> = collect_stream(stream).await?;
        let mut entities = Vec::new();
        for batch in &batches {
            for row in 0..batch.num_rows() {
                if let Ok(Some(e)) = entity_from_batch(batch, row) {
                    entities.push(e);
                }
            }
        }
        entities.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entities.truncate(limit);
        Ok(entities)
    }

    /// Text search across entities: case-insensitive substring match on value/normalized.
    pub async fn search_entities_text(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        let table = self.connection.open_table("entities").execute().await?;
        let ql = query.to_lowercase();

        let stream = table.query().limit(20_000).execute().await?;
        let batches: Vec<RecordBatch> = collect_stream(stream).await?;
        let mut matched = Vec::new();
        for batch in &batches {
            for row in 0..batch.num_rows() {
                if let Ok(Some(e)) = entity_from_batch(batch, row) {
                    let ev = e.value.to_lowercase();
                    let en = e.normalized.to_lowercase();
                    // Bidirectional match: query-in-entity-value handles direct lookups;
                    // entity-value-in-query handles natural language ("Who is Bob Matsuoka?").
                    // Require entity value >= 4 chars to avoid noisy single-word matches.
                    let reverse_match = ev.len() >= 4 && (ql.contains(&ev) || ql.contains(&en));
                    if ev.contains(&ql) || en.contains(&ql) || reverse_match {
                        matched.push(e);
                        if matched.len() >= limit {
                            return Ok(matched);
                        }
                    }
                }
            }
        }
        Ok(matched)
    }

    /// Delete a memory by ID.
    pub async fn delete_memory(&self, id: &str) -> Result<()> {
        let table = self.connection.open_table("memories").execute().await?;
        table.delete(&format!("id = '{}'", id)).await?;
        Ok(())
    }
}

// --- helpers ---

fn entity_type_str(t: &EntityType) -> &'static str {
    match t {
        EntityType::Person => "Person",
        EntityType::Company => "Company",
        EntityType::Project => "Project",
        EntityType::Tool => "Tool",
        EntityType::Topic => "Topic",
        EntityType::Location => "Location",
        EntityType::ActionItem => "ActionItem",
    }
}

fn entity_type_from_str(s: &str) -> EntityType {
    match s {
        "Company" => EntityType::Company,
        "Project" => EntityType::Project,
        "Tool" => EntityType::Tool,
        "Topic" => EntityType::Topic,
        "Location" => EntityType::Location,
        "ActionItem" => EntityType::ActionItem,
        _ => EntityType::Person,
    }
}

fn memory_category_str(c: &MemoryCategory) -> &'static str {
    match c {
        MemoryCategory::UserPreference => "user_preference",
        MemoryCategory::PersonFact => "person_fact",
        MemoryCategory::ProjectFact => "project_fact",
        MemoryCategory::CompanyFact => "company_fact",
        MemoryCategory::RecurringEvent => "recurring_event",
        MemoryCategory::Decision => "decision",
        MemoryCategory::Event => "event",
        MemoryCategory::General => "general",
    }
}

fn memory_category_from_str(s: &str) -> MemoryCategory {
    match s {
        "user_preference" => MemoryCategory::UserPreference,
        "person_fact" => MemoryCategory::PersonFact,
        "project_fact" => MemoryCategory::ProjectFact,
        "company_fact" => MemoryCategory::CompanyFact,
        "recurring_event" => MemoryCategory::RecurringEvent,
        "decision" => MemoryCategory::Decision,
        "event" => MemoryCategory::Event,
        _ => MemoryCategory::General,
    }
}

fn entity_from_batch(batch: &RecordBatch, row: usize) -> Result<Option<Entity>> {
    let get_str = |name: &str| -> String {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row).to_string())
            .unwrap_or_default()
    };
    let get_f32 = |name: &str| -> f32 {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
            .map(|a| a.value(row))
            .unwrap_or(0.0)
    };
    let get_i32 = |name: &str| -> i32 {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Int32Array>())
            .map(|a| a.value(row))
            .unwrap_or(0)
    };

    // Canonical field: "value". Legacy fallback: "name" (old Rust schema).
    let value = {
        let v = get_str("value");
        if v.is_empty() {
            get_str("name")
        } else {
            v
        }
    };
    let normalized = get_str("normalized");
    let entity_type_str_val = get_str("entity_type");
    let user_id = get_str("user_id");

    // Skip entirely blank rows.
    if value.is_empty() && normalized.is_empty() {
        return Ok(None);
    }

    // Canonical schema always has `id`. Generate synthetically only as fallback.
    let id_raw = get_str("id");
    let id = if !id_raw.is_empty() {
        uuid::Uuid::parse_str(&id_raw)
            .unwrap_or_else(|_| synthetic_entity_id(&entity_type_str_val, &normalized, &user_id))
    } else {
        synthetic_entity_id(&entity_type_str_val, &normalized, &user_id)
    };

    let confidence = {
        let c = get_f32("confidence");
        if c == 0.0 {
            1.0
        } else {
            c
        }
    };

    let first_seen = parse_dt(&get_str("first_seen"))
        .or_else(|| parse_dt(&get_str("created_at")))
        .unwrap_or_else(chrono::Utc::now);
    let last_seen = parse_dt(&get_str("last_seen")).unwrap_or(first_seen);

    // Canonical: "aliases" JSON. Legacy fallback: "attributes" JSON.
    let aliases_json = {
        let a = get_str("aliases");
        if a.is_empty() {
            get_str("attributes")
        } else {
            a
        }
    };
    let aliases: Vec<String> = serde_json::from_str(&aliases_json).unwrap_or_default();

    // Canonical: "occurrence_count" Int32. Legacy fallback handled by get_i32 returning 0.
    let occurrence_count = {
        let v = get_i32("occurrence_count");
        if v > 0 {
            v as u32
        } else {
            1
        }
    };

    let source = {
        let s = get_str("source");
        if s.is_empty() {
            "lance".to_string()
        } else {
            s
        }
    };
    // Canonical: "source_id" plain string. Legacy: "source_emails" JSON array.
    let source_id = {
        let s = get_str("source_id");
        if s.is_empty() {
            let emails_json = get_str("source_emails");
            let emails: Vec<String> = serde_json::from_str(&emails_json).unwrap_or_default();
            emails.into_iter().next().filter(|e| !e.is_empty())
        } else {
            Some(s)
        }
    };
    let context = {
        let s = get_str("context");
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };

    Ok(Some(Entity {
        id,
        user_id,
        entity_type: entity_type_from_str(&entity_type_str_val),
        value,
        normalized,
        confidence,
        source,
        source_id,
        context,
        aliases,
        occurrence_count,
        first_seen,
        last_seen,
        created_at: first_seen,
    }))
}

fn memory_from_batch(batch: &RecordBatch, row: usize) -> Result<Option<Memory>> {
    let get_str = |name: &str| -> String {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row).to_string())
            .unwrap_or_default()
    };
    let get_f32 = |name: &str| -> f32 {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
            .map(|a| a.value(row))
            .unwrap_or(0.0)
    };
    let get_bool = |name: &str| -> bool {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
            .map(|a| a.value(row))
            .unwrap_or(false)
    };

    let content = get_str("content");
    if content.is_empty() {
        return Ok(None);
    }

    // Canonical: is_deleted. Legacy fallback: archived.
    if get_bool("is_deleted") || get_bool("archived") {
        return Ok(None);
    }

    let category_str = get_str("category");
    let user_id = get_str("user_id");

    // Canonical schema always has `id`. Generate synthetically only as fallback.
    let id_raw = get_str("id");
    let id = if !id_raw.is_empty() {
        uuid::Uuid::parse_str(&id_raw)
            .unwrap_or_else(|_| synthetic_memory_id(&content, &category_str))
    } else {
        synthetic_memory_id(&content, &category_str)
    };

    let created_at = parse_dt(&get_str("created_at")).unwrap_or_else(chrono::Utc::now);
    let last_accessed = parse_dt(&get_str("last_accessed"));

    // Canonical: "related_entities" JSON. Legacy fallback: "tags" JSON.
    let entities_json = {
        let re = get_str("related_entities");
        if re.is_empty() {
            get_str("tags")
        } else {
            re
        }
    };
    let related_entities: Vec<String> = serde_json::from_str(&entities_json).unwrap_or_default();

    let source_id = {
        let s = get_str("source_id");
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };

    Ok(Some(Memory {
        id,
        user_id,
        category: memory_category_from_str(&category_str),
        content,
        embedding: None,
        related_entities,
        source_id,
        importance: get_f32("importance"),
        access_count: 0,
        last_accessed,
        created_at,
        updated_at: created_at,
    }))
}

/// Generate a stable UUID from `entity_type`, `normalized`, and `user_id`.
fn synthetic_entity_id(entity_type: &str, normalized: &str, user_id: &str) -> uuid::Uuid {
    let key = format!("{}:{}:{}", entity_type, normalized, user_id);
    let hash = Sha256::digest(key.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes)
}

/// Generate a stable UUID from `content` (truncated) and `category`.
fn synthetic_memory_id(content: &str, category: &str) -> uuid::Uuid {
    let n = content.len().min(64);
    let key = format!("{}:{}", category, &content[..n]);
    let hash = Sha256::digest(key.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes)
}

/// Parse an RFC-3339 datetime string, returning `None` on empty or invalid input.
fn parse_dt(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if s.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}
