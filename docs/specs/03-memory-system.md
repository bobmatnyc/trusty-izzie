# Memory System

## Overview

The memory system is distinct from the entity knowledge graph. While the knowledge graph stores structured facts about people, companies, and projects, the memory system stores the user's observations, preferences, and episodic context — the kind of information that makes an assistant feel personalized rather than generic.

Memories are discovered during chat: the LLM identifies notable facts from the conversation and returns them in the structured response as `memoriesToSave[]`. The memory manager persists these with embeddings, and retrieves relevant ones on future chat turns to enrich context.

Memories decay over time. Rarely-accessed memories fade; frequently-referenced ones stay strong. This mirrors human memory and prevents the context window from filling with stale information.

---

## Memory Schema

### Rust struct (`trusty-models`)

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id:           String,           // UUID v4
    pub user_id:      String,           // fixed single user
    pub content:      String,           // the memory text, max 500 chars
    pub category:     MemoryCategory,
    pub confidence:   f32,              // 0.0–1.0, how certain we are this is accurate
    pub importance:   f32,              // 0.0–1.0, user-assigned or LLM-estimated
    pub strength:     f32,              // 0.0–1.0, current decayed value (starts at 1.0)
    pub decay_rate:   f32,              // per-day decay rate (derived from category)
    pub created_at:   DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,  // updated on retrieval
    pub source:       MemorySource,    // where this memory originated
    pub entity_refs:  Vec<String>,     // entity IDs this memory relates to
    pub tags:         Vec<String>,     // optional free-form tags
    pub embedding:    Vec<f32>,        // 384-dim, stored in LanceDB
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryCategory {
    Preference,    // decay: 0.01/day — slow (user preferences rarely change)
    Fact,          // decay: 0.02/day — slow-medium (stable facts)
    Relationship,  // decay: 0.02/day — slow-medium (relationship context)
    Decision,      // decay: 0.03/day — medium (decisions become outdated)
    Event,         // decay: 0.05/day — medium-fast (past events become less relevant)
    Sentiment,     // decay: 0.10/day — fast (sentiments shift)
    Reminder,      // decay: 0.20/day — very fast (time-sensitive)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemorySource {
    ChatConversation { session_id: String },
    EmailExtraction  { message_id: String },
    UserExplicit,    // user directly told the assistant to remember something
}
```

---

## Decay Rates by Category

| Category     | Decay Rate (per day) | Half-life (importance=0) | Rationale                              |
|--------------|---------------------|--------------------------|----------------------------------------|
| Preference   | 0.01                | ~69 days                 | Stable: how someone prefers to work    |
| Fact         | 0.02                | ~35 days                 | Stable facts about the world           |
| Relationship | 0.02                | ~35 days                 | Relationships change slowly            |
| Decision     | 0.03                | ~23 days                 | Decisions eventually get superseded    |
| Event        | 0.05                | ~14 days                 | Past events lose relevance quickly     |
| Sentiment    | 0.10                | ~7 days                  | Mood and sentiment shift frequently    |
| Reminder     | 0.20                | ~3.5 days                | Reminders are inherently time-bound    |

The `importance` field modulates decay: high-importance memories decay slower.

---

## Temporal Decay Formula

```
strength(t) = exp(-decay_rate × days × (1 - importance × 0.5))
```

Where:
- `decay_rate` = category-specific rate (table above)
- `days` = number of days since last access (not creation)
- `importance` = 0.0–1.0 (0.5 default if not specified)

**Examples:**

A `Preference` memory (rate=0.01) with importance=0.5, 30 days since last access:
```
strength = exp(-0.01 × 30 × (1 - 0.5 × 0.5))
         = exp(-0.01 × 30 × 0.75)
         = exp(-0.225)
         ≈ 0.80
```

A `Reminder` memory (rate=0.20) with importance=0.2, 7 days since last access:
```
strength = exp(-0.20 × 7 × (1 - 0.2 × 0.5))
         = exp(-0.20 × 7 × 0.90)
         = exp(-1.26)
         ≈ 0.28
```

**Archival threshold:** Memories with `strength < 0.05` are moved to an archived state (not deleted — can be recovered). They are excluded from context retrieval.

---

## Composite Ranking Formula

When retrieving memories for context, results from hybrid search are re-ranked using:

```
score = strength × 0.5 + confidence × 0.3 + importance × 0.2
```

This ensures that:
- Decayed memories don't dominate even if semantically similar (strength weight)
- Low-confidence memories (uncertain facts) rank below confident ones
- User-flagged important memories get a boost

The hybrid search (BM25 + vector via RRF) produces an initial candidate set of N memories. The composite score is applied to re-rank these candidates, and the top-K are selected for context injection.

---

## Context Retrieval Pipeline

```rust
pub async fn retrieve_context(
    query: &str,
    limit: usize,            // default: 8 memories for context
    store: &Store,
    engine: &EmbeddingEngine,
    search: &HybridSearchEngine,
) -> TrustyResult<Vec<RankedMemory>> {

    // 1. Embed the query
    let query_vec = engine.embed_one(query).await?;

    // 2. Vector search over LanceDB memories table
    let vector_results = store.memories()
        .vector_search(query_vec.clone(), limit * 3)
        .await?;

    // 3. BM25 search over tantivy (memories are also indexed)
    let bm25_results = store.memories()
        .bm25_search(query, limit * 3)
        .await?;

    // 4. RRF fusion
    let fused = search.fuse(
        &vector_results,
        &bm25_results,
        0.7,   // alpha: vector weight
        60,    // K constant
    );

    // 5. Filter out archived memories (strength < 0.05)
    let active: Vec<_> = fused.iter()
        .filter(|m| m.strength >= 0.05)
        .take(limit * 2)
        .collect();

    // 6. Compute composite score and re-rank
    let mut ranked: Vec<RankedMemory> = active.iter().map(|m| {
        let composite = m.strength * 0.5 + m.confidence * 0.3 + m.importance * 0.2;
        RankedMemory {
            memory: m.clone(),
            composite_score: composite,
        }
    }).collect();
    ranked.sort_by(|a, b| b.composite_score.partial_cmp(&a.composite_score).unwrap());

    // 7. Take top-K
    let top_k = ranked.into_iter().take(limit).collect::<Vec<_>>();

    // 8. Refresh last_accessed for retrieved memories (access boost)
    for rm in &top_k {
        store.memories().refresh_access(&rm.memory.id).await?;
    }

    Ok(top_k)
}
```

**Access refresh behavior:** Accessing a memory resets `last_accessed` to now, which resets the decay clock. Frequently retrieved memories effectively never decay, as their `days` counter keeps resetting. This is intentional — relevant memories stay strong.

---

## Memory Consolidation

Memory consolidation runs on a schedule (default: once per day) to merge near-duplicate memories. This prevents the memory store from growing unboundedly with slight variations of the same fact.

```rust
pub async fn consolidate(
    store: &Store,
    engine: &EmbeddingEngine,
) -> TrustyResult<ConsolidationReport> {
    let all_memories = store.memories().list_above_threshold(0.05).await?;
    let mut merged = 0u64;

    for i in 0..all_memories.len() {
        for j in (i+1)..all_memories.len() {
            let m1 = &all_memories[i];
            let m2 = &all_memories[j];

            // Same category is required for merging
            if m1.category != m2.category { continue; }

            // Cosine similarity between embeddings
            let similarity = cosine_similarity(&m1.embedding, &m2.embedding);

            // Threshold: > 0.92 cosine similarity = near-duplicate
            if similarity > 0.92 {
                // Merge: keep the one with higher importance, combine confidence
                // as weighted average, reset strength to max of the two
                let winner = if m1.importance >= m2.importance { m1 } else { m2 };
                let loser  = if m1.importance >= m2.importance { m2 } else { m1 };

                let merged_confidence = (winner.confidence + loser.confidence) / 2.0;
                let merged_strength   = f32::max(winner.strength, loser.strength);

                store.memories().merge(
                    &winner.id,
                    merged_confidence,
                    merged_strength,
                    &loser.id,  // archive the loser
                ).await?;

                merged += 1;
            }
        }
    }

    Ok(ConsolidationReport {
        merged,
        kept: all_memories.len() as u64 - merged,
    })
}
```

**Merge rules:**
- Keep the memory with higher importance
- Merged confidence = average of both
- Merged strength = max of both (more generous — we don't want to lose strong memories)
- The "loser" is archived (soft delete), not hard deleted

---

## Decay Job

The decay job runs after every email sync cycle (driven by `trusty-daemon`):

```rust
pub async fn run_decay(store: &Store) -> TrustyResult<DecayReport> {
    let now = Utc::now();
    let all = store.memories().list_all_active().await?;
    let mut updated = 0u64;
    let mut archived = 0u64;

    for memory in all {
        let days = (now - memory.last_accessed).num_seconds() as f32 / 86400.0;
        let new_strength = f32::exp(
            -memory.decay_rate * days * (1.0 - memory.importance * 0.5)
        );

        if new_strength < 0.05 {
            store.memories().archive(&memory.id).await?;
            archived += 1;
        } else if (new_strength - memory.strength).abs() > 0.001 {
            // Only write if strength changed meaningfully
            store.memories().update_strength(&memory.id, new_strength).await?;
            updated += 1;
        }
    }

    Ok(DecayReport { updated, archived })
}
```

---

## LanceDB Schema for Memories Table

The LanceDB table name: `{user_id}_memories`

```rust
// Arrow schema for LanceDB
// (LanceDB uses Apache Arrow columnar format)

Field::new("id",            DataType::Utf8,   false),
Field::new("user_id",       DataType::Utf8,   false),
Field::new("content",       DataType::Utf8,   false),
Field::new("category",      DataType::Utf8,   false),   // enum as string
Field::new("confidence",    DataType::Float32, false),
Field::new("importance",    DataType::Float32, false),
Field::new("strength",      DataType::Float32, false),
Field::new("decay_rate",    DataType::Float32, false),
Field::new("created_at",    DataType::Utf8,   false),   // ISO 8601
Field::new("last_accessed", DataType::Utf8,   false),   // ISO 8601
Field::new("source",        DataType::Utf8,   false),   // JSON-encoded MemorySource
Field::new("entity_refs",   DataType::Utf8,   false),   // JSON array of entity IDs
Field::new("tags",          DataType::Utf8,   false),   // JSON array of strings
Field::new("archived",      DataType::Boolean, false),  // soft delete flag
Field::new("embedding",     DataType::FixedSizeList(
    Arc::new(Field::new("item", DataType::Float32, false)), 384
), false),
```

**Index configuration:**
- IVF-PQ index on `embedding` column (for ANN search)
- `num_partitions = 256` (suitable for up to ~100K memories)
- `num_sub_vectors = 32`

**Filter queries (LanceDB SQL-like):**
```sql
-- Retrieve active memories for user
SELECT * FROM {user_id}_memories
WHERE archived = false AND strength >= 0.05
ORDER BY last_accessed DESC
LIMIT 1000
```

---

## Memory-to-Chat Integration

During chat, memories are retrieved and formatted for the system prompt:

```rust
pub fn format_memories_for_context(memories: &[RankedMemory]) -> String {
    if memories.is_empty() {
        return String::new();
    }

    let mut lines = vec!["## Relevant memories from past conversations:".to_string()];
    for (i, rm) in memories.iter().enumerate() {
        lines.push(format!(
            "{}. [{}] {} (confidence: {:.0}%, strength: {:.0}%)",
            i + 1,
            rm.memory.category.display_name(),
            rm.memory.content,
            rm.memory.confidence * 100.0,
            rm.memory.strength * 100.0,
        ));
    }
    lines.join("\n")
}
```

**Example output injected into system prompt:**
```
## Relevant memories from past conversations:
1. [Preference] Alice prefers async communication over meetings (confidence: 92%, strength: 85%)
2. [Fact] The Prometheus project deadline is Q2 2026 (confidence: 88%, strength: 71%)
3. [Relationship] Bob reports to Carol in the infrastructure team (confidence: 95%, strength: 90%)
```

---

## Memory Saving (from Chat Response)

The LLM returns memories to save in the structured chat response:

```json
{
  "message": "...",
  "currentTask": "answering question about Alice",
  "memoriesToSave": [
    {
      "content": "Alice prefers bullet-point status updates rather than prose",
      "category": "preference",
      "confidence": 0.9,
      "importance": 0.7,
      "entity_refs": ["entity-id-for-alice"]
    }
  ]
}
```

The chat engine persists each `memoriesToSave` item:
```rust
for mem in turn_result.memories_to_save {
    let embedding = engine.embed_one(&mem.content).await?;
    let memory = Memory {
        id: Uuid::new_v4().to_string(),
        user_id: session.user_id.clone(),
        content: mem.content,
        category: mem.category.into(),
        confidence: mem.confidence,
        importance: mem.importance,
        strength: 1.0,  // starts at full strength
        decay_rate: mem.category.decay_rate(),
        created_at: Utc::now(),
        last_accessed: Utc::now(),
        source: MemorySource::ChatConversation { session_id: session.id.clone() },
        entity_refs: mem.entity_refs,
        tags: vec![],
        embedding,
    };
    memory_manager.save(&memory, store, engine).await?;
}
```
