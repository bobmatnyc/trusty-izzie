//! LanceDB vector store for memory and entity embeddings.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use arrow_array::{
    Array, BooleanArray, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator,
    StringArray, UInt32Array,
};
use arrow_schema::{DataType, Field, Schema};
use futures::Stream;
use lancedb::query::{ExecutableQuery, QueryBase};
use serde_json::Value as JsonValue;
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

fn entity_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("normalized", DataType::Utf8, false),
        Field::new("entity_type", DataType::Utf8, false),
        Field::new("confidence", DataType::Float32, false),
        Field::new("seen_count", DataType::UInt32, false),
        Field::new("attributes", DataType::Utf8, false),
        Field::new("first_seen", DataType::Utf8, false),
        Field::new("last_seen", DataType::Utf8, false),
        Field::new("source_emails", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_DIM,
            ),
            false,
        ),
    ]))
}

fn memory_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("content", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("confidence", DataType::Float32, false),
        Field::new("importance", DataType::Float32, false),
        Field::new("strength", DataType::Float32, false),
        Field::new("decay_rate", DataType::Float32, false),
        Field::new("created_at", DataType::Utf8, false),
        Field::new("last_accessed", DataType::Utf8, false),
        Field::new("tags", DataType::Utf8, false),
        Field::new("archived", DataType::Boolean, false),
        Field::new(
            "embedding",
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
    // Find the embedding field's list type
    let emb_field = schema
        .field_with_name("embedding")
        .map_err(|e| anyhow!("schema missing embedding: {}", e))?;
    let item_field = match emb_field.data_type() {
        DataType::FixedSizeList(f, _) => f.clone(),
        _ => return Err(anyhow!("unexpected embedding type")),
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
        let attributes = serde_json::to_string(&entity.aliases)?;
        let source_emails = entity.source_id.as_deref().unwrap_or("").to_string();
        let source_emails_json = serde_json::json!([source_emails]).to_string();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id_str.as_str()])),
                Arc::new(StringArray::from(vec![entity.value.as_str()])),
                Arc::new(StringArray::from(vec![entity.normalized.as_str()])),
                Arc::new(StringArray::from(vec![entity_type_str(
                    &entity.entity_type,
                )])),
                Arc::new(Float32Array::from(vec![entity.confidence])),
                Arc::new(UInt32Array::from(vec![entity.occurrence_count])),
                Arc::new(StringArray::from(vec![attributes.as_str()])),
                Arc::new(StringArray::from(vec![entity
                    .first_seen
                    .to_rfc3339()
                    .as_str()])),
                Arc::new(StringArray::from(vec![entity
                    .last_seen
                    .to_rfc3339()
                    .as_str()])),
                Arc::new(StringArray::from(vec![source_emails_json.as_str()])),
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
        let tags_json = serde_json::json!(memory.related_entities).to_string();
        let last_accessed = memory
            .last_accessed
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| memory.created_at.to_rfc3339());
        let decay_rate = 1.0_f32 / (memory.category.decay_half_life_days() * 86400.0);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id_str.as_str()])),
                Arc::new(StringArray::from(vec![memory.content.as_str()])),
                Arc::new(StringArray::from(vec![memory_category_str(
                    &memory.category,
                )])),
                Arc::new(Float32Array::from(vec![1.0_f32])), // confidence
                Arc::new(Float32Array::from(vec![memory.importance])),
                Arc::new(Float32Array::from(vec![1.0_f32])), // strength (undecayed)
                Arc::new(Float32Array::from(vec![decay_rate])),
                Arc::new(StringArray::from(vec![memory
                    .created_at
                    .to_rfc3339()
                    .as_str()])),
                Arc::new(StringArray::from(vec![last_accessed.as_str()])),
                Arc::new(StringArray::from(vec![tags_json.as_str()])),
                Arc::new(BooleanArray::from(vec![false])),
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
    let get_str = |name: &str| -> Result<&str> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row))
            .ok_or_else(|| anyhow!("missing column '{}'", name))
    };
    let get_f32 = |name: &str| -> Result<f32> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
            .map(|a| a.value(row))
            .ok_or_else(|| anyhow!("missing column '{}'", name))
    };
    let get_u32 = |name: &str| -> Result<u32> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<UInt32Array>())
            .map(|a| a.value(row))
            .ok_or_else(|| anyhow!("missing column '{}'", name))
    };

    let id_str = get_str("id")?;
    let id =
        uuid::Uuid::parse_str(id_str).map_err(|e| anyhow!("invalid uuid '{}': {}", id_str, e))?;

    let first_seen = chrono::DateTime::parse_from_rfc3339(get_str("first_seen")?)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let last_seen = chrono::DateTime::parse_from_rfc3339(get_str("last_seen")?)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());

    let aliases: Vec<String> = serde_json::from_str(get_str("attributes")?).unwrap_or_default();
    let source_emails: Vec<String> =
        serde_json::from_str(get_str("source_emails")?).unwrap_or_default();
    let source_id = source_emails.into_iter().next().filter(|s| !s.is_empty());

    Ok(Some(Entity {
        id,
        user_id: String::new(),
        entity_type: entity_type_from_str(get_str("entity_type")?),
        value: get_str("name")?.to_string(),
        normalized: get_str("normalized")?.to_string(),
        confidence: get_f32("confidence")?,
        source: "lance".to_string(),
        source_id,
        context: None,
        aliases,
        occurrence_count: get_u32("seen_count")?,
        first_seen,
        last_seen,
        created_at: first_seen,
    }))
}

fn memory_from_batch(batch: &RecordBatch, row: usize) -> Result<Option<Memory>> {
    let get_str = |name: &str| -> Result<&str> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| a.value(row))
            .ok_or_else(|| anyhow!("missing column '{}'", name))
    };
    let get_f32 = |name: &str| -> Result<f32> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
            .map(|a| a.value(row))
            .ok_or_else(|| anyhow!("missing column '{}'", name))
    };

    let id_str = get_str("id")?;
    let id =
        uuid::Uuid::parse_str(id_str).map_err(|e| anyhow!("invalid uuid '{}': {}", id_str, e))?;

    let created_at = chrono::DateTime::parse_from_rfc3339(get_str("created_at")?)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let last_accessed = chrono::DateTime::parse_from_rfc3339(get_str("last_accessed")?)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .ok();

    let tags: Vec<String> = serde_json::from_str(get_str("tags")?).unwrap_or_default();

    Ok(Some(Memory {
        id,
        user_id: String::new(),
        category: memory_category_from_str(get_str("category")?),
        content: get_str("content")?.to_string(),
        embedding: None,
        related_entities: tags,
        source_id: None,
        importance: get_f32("importance")?,
        access_count: 0,
        last_accessed,
        created_at,
        updated_at: created_at,
    }))
}

// Silence unused import warning for JsonValue (used via serde_json::json!)
const _: () = {
    let _ = std::mem::size_of::<JsonValue>();
};
