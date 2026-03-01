# Storage Layer

## Overview

trusty-izzie uses four complementary storage backends, each chosen for a specific access pattern. They are initialized together at startup and accessed through the unified `Store` struct in `trusty-store`.

| Backend   | Role                              | Location                            |
|-----------|-----------------------------------|-------------------------------------|
| LanceDB   | Vector embeddings + ANN search    | `{data_dir}/lancedb/`               |
| Kuzu      | Graph relationships               | `{data_dir}/kuzu/`                  |
| SQLite    | Auth, cursors, sessions, config   | `{data_dir}/trusty.db`              |
| tantivy   | BM25 full-text search             | `{data_dir}/tantivy/`               |

Default `data_dir`: `~/.trusty-izzie/`

---

## LanceDB Tables

LanceDB stores vector embeddings using Apache Arrow columnar format. Each table is prefixed with `{user_id}_` for future multi-user extensibility (in v1, `user_id` is always the value from config).

### `{user_id}_entities`

Stores entity vectors and all associated metadata.

```rust
// Arrow schema
Schema::new(vec![
    Field::new("id",              DataType::Utf8,    false),  // UUID v4
    Field::new("user_id",         DataType::Utf8,    false),
    Field::new("name",            DataType::Utf8,    false),  // canonical name
    Field::new("normalized_name", DataType::Utf8,    false),  // lowercased, no punctuation
    Field::new("entity_type",     DataType::Utf8,    false),  // "Person"|"Company"|...
    Field::new("confidence",      DataType::Float32, false),  // extraction confidence
    Field::new("seen_count",      DataType::UInt32,  false),  // # emails this appeared in
    Field::new("attributes",      DataType::Utf8,    false),  // JSON object (type-specific)
    Field::new("first_seen",      DataType::Utf8,    false),  // ISO 8601 datetime
    Field::new("last_seen",       DataType::Utf8,    false),  // ISO 8601 datetime
    Field::new("source_emails",   DataType::Utf8,    false),  // JSON array of message_ids
    Field::new("searchable_text", DataType::Utf8,    false),  // denormalized for BM25
    Field::new("embedding",       DataType::FixedSizeList(
        Arc::new(Field::new("item", DataType::Float32, false)), 384
    ), false),
])
```

**Entity attributes by type (JSON):**

```json
// Person
{ "email": "alice@example.com", "role": "Engineering Manager", "company": "Acme Corp" }

// Company
{ "domain": "acme.com", "industry": "Software" }

// Project
{ "status": "active", "description": "Internal ML pipeline refactor" }

// Tool
{ "category": "Version control", "url": "https://github.com" }

// Topic
{ "description": "Distributed systems design" }

// Location
{ "country": "US", "city": "San Francisco" }

// ActionItem
{ "due_date": "2026-03-15", "assignee": "alice@example.com", "description": "Send Q1 report" }
```

**Index:**
```rust
// IVF-PQ index for ANN search
let index = IvfPqIndexBuilder::default()
    .num_partitions(256)
    .num_sub_vectors(32)
    .build();
table.create_index(&["embedding"], index).await?;
```

**Common queries:**
```rust
// Vector similarity search
table.search(query_vec)
    .filter("user_id = 'user1'")
    .limit(20)
    .execute()
    .await?;

// Lookup by normalized name (dedup)
table.query()
    .filter("user_id = 'user1' AND normalized_name = 'acme corp'")
    .execute()
    .await?;

// List by type
table.query()
    .filter("user_id = 'user1' AND entity_type = 'Person'")
    .limit(100)
    .offset(0)
    .execute()
    .await?;
```

---

### `{user_id}_memories`

```rust
Schema::new(vec![
    Field::new("id",           DataType::Utf8,    false),  // UUID v4
    Field::new("user_id",      DataType::Utf8,    false),
    Field::new("content",      DataType::Utf8,    false),  // max 500 chars
    Field::new("category",     DataType::Utf8,    false),  // "preference"|"fact"|...
    Field::new("confidence",   DataType::Float32, false),
    Field::new("importance",   DataType::Float32, false),
    Field::new("strength",     DataType::Float32, false),  // current decayed value
    Field::new("decay_rate",   DataType::Float32, false),
    Field::new("created_at",   DataType::Utf8,    false),
    Field::new("last_accessed",DataType::Utf8,    false),
    Field::new("source",       DataType::Utf8,    false),  // JSON-encoded MemorySource
    Field::new("entity_refs",  DataType::Utf8,    false),  // JSON array of entity IDs
    Field::new("tags",         DataType::Utf8,    false),  // JSON array
    Field::new("archived",     DataType::Boolean, false),
    Field::new("embedding",    DataType::FixedSizeList(
        Arc::new(Field::new("item", DataType::Float32, false)), 384
    ), false),
])
```

---

### `{user_id}_research`

Stores research findings associated with entities or topics. Used when the user asks the assistant to research something and the result should be persisted.

```rust
Schema::new(vec![
    Field::new("id",          DataType::Utf8,    false),
    Field::new("user_id",     DataType::Utf8,    false),
    Field::new("title",       DataType::Utf8,    false),
    Field::new("content",     DataType::Utf8,    false),  // research summary
    Field::new("entity_refs", DataType::Utf8,    false),  // JSON array of entity IDs
    Field::new("source_url",  DataType::Utf8,    true),   // nullable
    Field::new("created_at",  DataType::Utf8,    false),
    Field::new("embedding",   DataType::FixedSizeList(
        Arc::new(Field::new("item", DataType::Float32, false)), 384
    ), false),
])
```

---

## Kuzu Graph Schema

Kuzu uses its own Cypher-like query language. The schema is defined using Kuzu's DDL.

### Node Types

```cypher
CREATE NODE TABLE Person (
    id          STRING,
    name        STRING,
    email       STRING,
    role        STRING,
    confidence  FLOAT,
    first_seen  STRING,
    last_seen   STRING,
    PRIMARY KEY (id)
);

CREATE NODE TABLE Company (
    id          STRING,
    name        STRING,
    domain      STRING,
    industry    STRING,
    confidence  FLOAT,
    first_seen  STRING,
    last_seen   STRING,
    PRIMARY KEY (id)
);

CREATE NODE TABLE Project (
    id          STRING,
    name        STRING,
    status      STRING,
    description STRING,
    confidence  FLOAT,
    first_seen  STRING,
    last_seen   STRING,
    PRIMARY KEY (id)
);

CREATE NODE TABLE Tool (
    id          STRING,
    name        STRING,
    category    STRING,
    url         STRING,
    confidence  FLOAT,
    first_seen  STRING,
    last_seen   STRING,
    PRIMARY KEY (id)
);

CREATE NODE TABLE Topic (
    id          STRING,
    name        STRING,
    description STRING,
    confidence  FLOAT,
    first_seen  STRING,
    last_seen   STRING,
    PRIMARY KEY (id)
);

CREATE NODE TABLE Location (
    id          STRING,
    name        STRING,
    city        STRING,
    country     STRING,
    confidence  FLOAT,
    first_seen  STRING,
    last_seen   STRING,
    PRIMARY KEY (id)
);
```

### Edge Types

```cypher
-- Person WORKS_FOR Company
CREATE REL TABLE WORKS_FOR (
    FROM Person TO Company,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING,
    status      STRING  -- "active"|"former"|"unknown"
);

-- Person WORKS_WITH Person
CREATE REL TABLE WORKS_WITH (
    FROM Person TO Person,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);

-- Person WORKS_ON Project
CREATE REL TABLE WORKS_ON (
    FROM Person TO Project,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);

-- Person REPORTS_TO Person
CREATE REL TABLE REPORTS_TO (
    FROM Person TO Person,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);

-- Person LEADS Project
CREATE REL TABLE LEADS (
    FROM Person TO Project,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);

-- Person EXPERT_IN Topic
CREATE REL TABLE EXPERT_IN (
    FROM Person TO Topic,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);

-- Company/Person LOCATED_IN Location
CREATE REL TABLE LOCATED_IN (
    FROM Person TO Location,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);
-- (Also: Company LOCATED_IN Location — Kuzu supports multiple FROM types in v0.5+)

-- Company PARTNERS_WITH Company
CREATE REL TABLE PARTNERS_WITH (
    FROM Company TO Company,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);

-- Generic bidirectional
CREATE REL TABLE RELATED_TO (
    FROM Person TO Topic,
    confidence  FLOAT,
    evidence    STRING,
    first_seen  STRING,
    last_seen   STRING
);
```

### Example Kuzu Queries

```cypher
-- Get all coworkers of Alice
MATCH (p:Person {name: "Alice"})-[:WORKS_WITH]-(colleague:Person)
RETURN colleague.name, colleague.role;

-- Find everyone who works for Acme Corp
MATCH (p:Person)-[:WORKS_FOR]->(c:Company {name: "Acme Corp"})
RETURN p.name, p.role;

-- Get 2-hop neighborhood of a person
MATCH (p:Person {id: "uuid-here"})-[r*1..2]-(neighbor)
RETURN p, r, neighbor
LIMIT 50;

-- Find project leads
MATCH (p:Person)-[:LEADS]->(proj:Project)
WHERE proj.status = "active"
RETURN p.name, proj.name, proj.description;
```

---

## SQLite Tables

All SQLite DDL executed at startup via `rusqlite` with `PRAGMA journal_mode=WAL` for concurrent reads.

### `oauth_tokens`

```sql
CREATE TABLE IF NOT EXISTS oauth_tokens (
    account_id      TEXT PRIMARY KEY,
    access_token    TEXT NOT NULL,
    refresh_token   TEXT NOT NULL,
    token_type      TEXT NOT NULL DEFAULT 'Bearer',
    expires_at      TEXT NOT NULL,  -- ISO 8601
    scopes          TEXT NOT NULL,  -- space-separated
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### `email_cursors`

```sql
CREATE TABLE IF NOT EXISTS email_cursors (
    account_id      TEXT PRIMARY KEY,
    history_id      TEXT NOT NULL,   -- Gmail historyId (large integer as string)
    last_sync_at    TEXT NOT NULL,   -- ISO 8601
    emails_synced   INTEGER NOT NULL DEFAULT 0,
    full_sync_done  INTEGER NOT NULL DEFAULT 0  -- boolean: 1 if initial full sync complete
);
```

### `chat_sessions`

```sql
CREATE TABLE IF NOT EXISTS chat_sessions (
    id                  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL,
    title               TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    last_active_at      TEXT NOT NULL DEFAULT (datetime('now')),
    message_count       INTEGER NOT NULL DEFAULT 0,
    compressed_summary  TEXT,       -- NULL until compression occurs
    token_estimate      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_chat_sessions_user_id
    ON chat_sessions(user_id, last_active_at DESC);
```

### `chat_messages`

```sql
CREATE TABLE IF NOT EXISTS chat_messages (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    role        TEXT NOT NULL CHECK(role IN ('user','assistant','tool')),
    content     TEXT NOT NULL,
    tool_calls  TEXT,    -- JSON array of ToolCall objects, NULL if none
    tool_results TEXT,   -- JSON array of ToolResult objects, NULL if none
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    tokens      INTEGER  -- NULL if not tracked
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_session
    ON chat_messages(session_id, created_at ASC);
```

### `entity_fingerprints`

```sql
CREATE TABLE IF NOT EXISTS entity_fingerprints (
    fingerprint     TEXT PRIMARY KEY,  -- SHA-256 of "type:normalized_name" (first 16 bytes)
    entity_id       TEXT NOT NULL,     -- LanceDB entity ID (may be placeholder before graduation)
    entity_type     TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    first_seen      TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen       TEXT NOT NULL DEFAULT (datetime('now')),
    seen_count      INTEGER NOT NULL DEFAULT 1,
    graduated       INTEGER NOT NULL DEFAULT 0,  -- boolean: 1 = written to LanceDB + Kuzu
    is_spam         INTEGER NOT NULL DEFAULT 0   -- boolean: 1 = spam fingerprint, skip forever
);

CREATE INDEX IF NOT EXISTS idx_entity_fingerprints_entity_id
    ON entity_fingerprints(entity_id);
CREATE INDEX IF NOT EXISTS idx_entity_fingerprints_normalized
    ON entity_fingerprints(entity_type, normalized_name);
```

### `config`

```sql
CREATE TABLE IF NOT EXISTS config (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
```

**Common config keys:**
```
user_id               — fixed user identifier
default_chat_model    — OpenRouter model string
default_sync_interval — seconds between sync cycles
api_key               — local API key for HTTP API access
data_dir              — override for data directory
lancedb_s3_bucket     — S3 bucket if using S3 backend
```

### `reprocessing_jobs`

```sql
CREATE TABLE IF NOT EXISTS reprocessing_jobs (
    job_id       TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL,
    from_date    TEXT NOT NULL,
    to_date      TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending'
                 CHECK(status IN ('pending','running','done','failed')),
    progress     INTEGER NOT NULL DEFAULT 0,
    total        INTEGER NOT NULL DEFAULT 0,
    started_at   TEXT,
    completed_at TEXT,
    error        TEXT
);
```

### `calendar_cache`

```sql
CREATE TABLE IF NOT EXISTS calendar_cache (
    event_id    TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL,
    title       TEXT NOT NULL,
    start_time  TEXT NOT NULL,  -- ISO 8601
    end_time    TEXT NOT NULL,  -- ISO 8601
    all_day     INTEGER NOT NULL DEFAULT 0,
    location    TEXT,
    attendees   TEXT,           -- JSON array of email strings
    cached_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_calendar_cache_start
    ON calendar_cache(account_id, start_time ASC);
```

---

## tantivy Full-Text Search (BM25)

The tantivy index provides fast keyword search over entity content and memories.

### Schema Definition

```rust
use tantivy::schema::*;

pub fn build_entity_schema() -> Schema {
    let mut schema_builder = Schema::builder();
    schema_builder.add_text_field("entity_id",   STRING | STORED);
    schema_builder.add_text_field("user_id",     STRING | STORED);
    schema_builder.add_text_field("entity_type", STRING | STORED);
    schema_builder.add_text_field("content",     TEXT);   // indexed, not stored (saves space)
    schema_builder.build()
}

pub fn build_memory_schema() -> Schema {
    let mut schema_builder = Schema::builder();
    schema_builder.add_text_field("memory_id",  STRING | STORED);
    schema_builder.add_text_field("user_id",    STRING | STORED);
    schema_builder.add_text_field("category",   STRING | STORED);
    schema_builder.add_text_field("content",    TEXT);
    schema_builder.build()
}
```

**Tokenizer:** Default tokenizer (`simple`) — lowercase + split on whitespace and punctuation. Sufficient for entity names and memory content.

### Index Initialization

```rust
pub fn open_or_create_index(index_path: &Path, schema: Schema) -> TrustyResult<Index> {
    if index_path.exists() {
        Index::open_in_dir(index_path)
            .map_err(|e| TrustyError::SearchError(e.to_string()))
    } else {
        std::fs::create_dir_all(index_path)?;
        Index::create_in_dir(index_path, schema)
            .map_err(|e| TrustyError::SearchError(e.to_string()))
    }
}
```

### Search Integration with RRF

```rust
pub struct HybridSearchResult {
    pub id:           String,
    pub score:        f32,
    pub bm25_rank:    Option<usize>,
    pub vector_rank:  Option<usize>,
}

/// Reciprocal Rank Fusion
/// score = alpha/(K+vector_rank) + (1-alpha)/(K+bm25_rank)
/// K=60, alpha=0.7
pub fn rrf_fuse(
    vector_results: &[VectorResult],  // ordered by vector score
    bm25_results:   &[BM25Result],    // ordered by BM25 score
    alpha: f32,    // default: 0.7 (vector weight)
    k:     f32,    // default: 60.0
) -> Vec<HybridSearchResult> {
    let mut scores: HashMap<String, f32> = HashMap::new();
    let mut bm25_ranks: HashMap<String, usize> = HashMap::new();
    let mut vector_ranks: HashMap<String, usize> = HashMap::new();

    for (rank, result) in vector_results.iter().enumerate() {
        let rrf_score = alpha / (k + rank as f32 + 1.0);
        *scores.entry(result.id.clone()).or_insert(0.0) += rrf_score;
        vector_ranks.insert(result.id.clone(), rank + 1);
    }

    for (rank, result) in bm25_results.iter().enumerate() {
        let rrf_score = (1.0 - alpha) / (k + rank as f32 + 1.0);
        *scores.entry(result.id.clone()).or_insert(0.0) += rrf_score;
        bm25_ranks.insert(result.id.clone(), rank + 1);
    }

    let mut combined: Vec<HybridSearchResult> = scores.into_iter().map(|(id, score)| {
        HybridSearchResult {
            id: id.clone(),
            score,
            bm25_rank: bm25_ranks.get(&id).copied(),
            vector_rank: vector_ranks.get(&id).copied(),
        }
    }).collect();

    combined.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    combined
}
```

### Tantivy Commit Strategy

Tantivy writes are buffered and committed in batch:
- After each email sync cycle (all entities from the batch)
- Before any search query if uncommitted writes exist
- Explicitly via `POST /api/sync` admin endpoint

```rust
pub async fn commit_if_needed(writer: &mut IndexWriter, force: bool) -> TrustyResult<()> {
    if force || writer.needs_commit() {
        writer.commit()
            .map_err(|e| TrustyError::SearchError(e.to_string()))?;
    }
    Ok(())
}
```

---

## Store Initialization Sequence

On daemon startup, the `Store::new()` method initializes all backends in order:

```rust
impl Store {
    pub async fn new(config: &StorageConfig) -> TrustyResult<Self> {
        // 1. Ensure data directory exists
        std::fs::create_dir_all(&config.data_dir)?;

        // 2. SQLite — runs first to provide config for other backends
        let db = SqlitePool::new(&config.sqlite_path())?;
        db.run_migrations().await?;

        // 3. LanceDB — open or create tables
        let lance = lancedb::connect(&config.lancedb_path()).execute().await?;
        let entity_table = open_or_create_entity_table(&lance, &config.user_id).await?;
        let memory_table = open_or_create_memory_table(&lance, &config.user_id).await?;

        // 4. Kuzu — open or create graph DB
        let kuzu = KuzuDb::open(&config.kuzu_path())?;
        kuzu.run_schema_migrations()?;

        // 5. tantivy — open or create indices
        let entity_index = open_or_create_index(&config.entity_index_path(), build_entity_schema())?;
        let memory_index = open_or_create_index(&config.memory_index_path(), build_memory_schema())?;

        Ok(Store {
            db,
            lance,
            entity_table,
            memory_table,
            kuzu,
            entity_index,
            memory_index,
        })
    }
}
```

---

## Storage Configuration

```toml
# config/default.toml
[storage]
data_dir     = "~/.trusty-izzie"
user_id      = "default"

# LanceDB: optional S3 backend
# lancedb_s3_bucket = "my-bucket"
# lancedb_s3_region = "us-east-1"

# SQLite WAL mode — enabled automatically
# tantivy writer memory budget
tantivy_writer_heap_mb = 50
```

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub data_dir:            PathBuf,
    pub user_id:             String,
    pub lancedb_s3_bucket:   Option<String>,
    pub lancedb_s3_region:   Option<String>,
    pub tantivy_writer_heap_mb: u64,  // default: 50
}

impl StorageConfig {
    pub fn sqlite_path(&self) -> PathBuf {
        self.data_dir.join("trusty.db")
    }
    pub fn lancedb_path(&self) -> String {
        if let Some(bucket) = &self.lancedb_s3_bucket {
            format!("s3://{}/lancedb", bucket)
        } else {
            self.data_dir.join("lancedb").to_string_lossy().to_string()
        }
    }
    pub fn kuzu_path(&self) -> PathBuf {
        self.data_dir.join("kuzu")
    }
    pub fn entity_index_path(&self) -> PathBuf {
        self.data_dir.join("tantivy").join("entities")
    }
    pub fn memory_index_path(&self) -> PathBuf {
        self.data_dir.join("tantivy").join("memories")
    }
}
```
