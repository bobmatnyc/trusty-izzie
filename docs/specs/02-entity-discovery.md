# Entity Discovery Pipeline

## Overview

Entity discovery is the process by which trusty-izzie builds knowledge about the user's professional world from their sent email history. The pipeline is intentionally conservative — it is far better to miss an entity than to pollute the knowledge graph with noise.

The pipeline runs as a background task in `trusty-daemon` on a configurable schedule (default: every 30 minutes). It is incremental: only emails since the last sync cursor are processed. The user never interacts with this pipeline directly.

---

## Entity Types

| Type         | Description                                           | Source constraint          |
|--------------|-------------------------------------------------------|----------------------------|
| `Person`     | Individual human contacts                             | Headers only (From/To/CC)  |
| `Company`    | Organizations, businesses, institutions               | Body + headers             |
| `Project`    | Named projects, products, initiatives                 | Body                       |
| `Tool`       | Software tools, platforms, services                   | Body                       |
| `Topic`      | Subject areas, domains, themes                        | Body                       |
| `Location`   | Geographic locations (city, country, office)          | Body                       |
| `ActionItem` | Tasks or commitments extracted from email content     | Body                       |

`ActionItem` entities are ephemeral — they are surfaced in chat but decay rapidly (see memory system) and are not added to the knowledge graph as permanent nodes.

---

## Pipeline Stages

### Stage 1: Gmail Sync (SENT Only)

```rust
// trusty-email crate
pub struct SyncResult {
    pub emails: Vec<RawEmail>,
    pub new_cursor: HistoryId,
    pub has_more: bool,
    pub rate_limited: bool,
}

pub async fn sync_sent(
    client: &GmailClient,
    cursor: Option<HistoryId>,
    batch_size: u32,  // default: 50
) -> TrustyResult<SyncResult>
```

**Implementation:**
1. Call `GET https://gmail.googleapis.com/gmail/v1/users/me/history` with:
   - `startHistoryId` = cursor (or `None` for initial full sync)
   - `labelId` = `SENT`
   - `maxResults` = batch_size (50)
   - `historyTypes` = `["messageAdded"]`
2. For each `messageId` in history, call `GET .../messages/{id}?format=full`
3. Extract headers (From, To, CC, Subject, Date) and body text (prefer text/plain)
4. Strip quoted reply chains (everything after `> ` lines or `On ... wrote:`)
5. Store new `historyId` as cursor in `email_cursors` SQLite table
6. If `nextPageToken` is present, `has_more = true` — caller loops

**Cursor management:**
```sql
-- email_cursors table
INSERT OR REPLACE INTO email_cursors (account_id, history_id, last_sync_at)
VALUES (?1, ?2, ?3)
```

**Rate limit handling:**
- 429 → exponential backoff starting at 1s, max 60s
- 403 with `reason: "rateLimitExceeded"` → same backoff
- Token expiry (401) → call `refresh_token_if_needed()` and retry once

---

### Stage 2: Spam and Newsletter Filtering

Before spending tokens on extraction, each email is evaluated by a cheap LLM call (mistral-small) to determine if it is spam, marketing, or a newsletter. Emails with a spam score ≥ 0.3 are skipped entirely and marked in `entity_fingerprints` so they are not re-evaluated on future syncs.

**Spam detection prompt:**

```
You are a classifier. Given an email's subject and first 200 characters of body,
output a JSON object with a single field "spam_score" between 0.0 and 1.0.

Score 0.0 means: clearly a personal or professional email worth analyzing.
Score 1.0 means: clearly spam, marketing, automated notification, or newsletter.

Signals that increase the score:
- Unsubscribe links in body
- "no-reply" or "noreply" in sender address
- Subject contains: "sale", "% off", "limited time", "newsletter", "digest",
  "notification", "alert from", "invoice #", "receipt from"
- Body is clearly templated (many {{placeholders}} or generic language)
- No specific person-to-person communication

Return ONLY valid JSON. Example: {"spam_score": 0.8}

Subject: {subject}
From: {from_address}
Body preview: {body_preview}
```

**Decision logic:**
```rust
if spam_result.spam_score >= 0.3 {
    // Record fingerprint so we never re-evaluate this message
    store.record_spam_fingerprint(email.message_id).await?;
    return Ok(None);  // Skip extraction
}
```

---

### Stage 3: Entity Extraction Prompt

**Model:** `mistral/mistral-small` via OpenRouter (cheap, fast, sufficient for structured extraction)

**The full extraction prompt:**

```
You are an entity extraction engine for a personal knowledge graph. Analyze the
following email (sent by the user) and extract structured information.

CRITICAL RULES - VIOLATIONS DISQUALIFY THE ENTIRE OUTPUT:
1. Persons MUST come ONLY from email headers (From, To, CC). NEVER extract person
   names from the email body. If you find a name only in the body, skip it.
2. Companies, Projects, Tools, Topics, and Locations may be extracted from the body.
3. Assign confidence scores (0.0–1.0) strictly. Do NOT inflate confidence.
   - 1.0: Explicitly stated, unambiguous (e.g., "Alice from Acme Corp")
   - 0.9: Very clear from context
   - 0.85: Clear but requires minor inference
   - Below 0.85: Do not include in output
4. For WORKS_WITH relationships: both parties must be explicitly collaborating on
   something specific in this email. Mere co-mention is NOT sufficient evidence.
5. Maximum 3 relationships per email. Choose only the strongest.
6. Do NOT create relationships between persons just because they are all on the CC list.
7. If the email is a forwarded message, only process the outermost/latest message.
8. Normalize names: "Bob Smith" and "Robert Smith" are likely the same person.
   Output the most formal version.

EMAIL HEADERS:
From: {from}
To: {to}
CC: {cc}
Subject: {subject}
Date: {date}

EMAIL BODY (quoted replies removed):
{body}

Output a JSON object with this exact schema:
{
  "entities": [
    {
      "name": "string",               // canonical name
      "type": "Person|Company|Project|Tool|Topic|Location|ActionItem",
      "confidence": 0.0-1.0,
      "attributes": {
        // type-specific fields (see below)
      },
      "source": "header|body"         // where this entity was found
    }
  ],
  "relationships": [
    {
      "from_entity": "string",        // entity name (must appear in entities[])
      "to_entity": "string",          // entity name (must appear in entities[])
      "type": "WORKS_FOR|WORKS_WITH|WORKS_ON|REPORTS_TO|LEADS|EXPERT_IN|LOCATED_IN|PARTNERS_WITH|RELATED_TO",
      "confidence": 0.0-1.0,
      "evidence": "string"            // exact quote or paraphrase from email proving this relationship
    }
  ]
}

Type-specific attributes:
- Person: { "email": "string|null", "role": "string|null", "company": "string|null" }
- Company: { "domain": "string|null", "industry": "string|null" }
- Project: { "status": "active|completed|unknown", "description": "string|null" }
- Tool: { "category": "string|null", "url": "string|null" }
- Topic: { "description": "string|null" }
- Location: { "country": "string|null", "city": "string|null" }
- ActionItem: { "due_date": "string|null", "assignee": "string|null", "description": "string" }

Return ONLY valid JSON. Do not add commentary before or after.
```

---

### Stage 4: Response Parsing and Validation

```rust
pub struct EntityParser;

impl EntityParser {
    pub fn parse(raw: &str) -> TrustyResult<ExtractionOutput> {
        // 1. Strip any markdown code fences the LLM may have added
        let cleaned = Self::strip_code_fences(raw);

        // 2. Parse JSON
        let output: RawExtractionOutput = serde_json::from_str(&cleaned)
            .map_err(|e| TrustyError::ParseError(format!("LLM JSON parse failed: {e}")))?;

        // 3. Structural validation
        for entity in &output.entities {
            Self::validate_entity(entity)?;
        }
        for rel in &output.relationships {
            Self::validate_relationship(&rel, &output.entities)?;
        }

        Ok(output.into())
    }

    fn validate_entity(e: &RawEntity) -> TrustyResult<()> {
        // Source constraint: persons must come from headers
        if e.entity_type == EntityType::Person && e.source != "header" {
            return Err(TrustyError::ValidationError(
                format!("Person '{}' not from header — discarded", e.name)
            ));
        }
        // Confidence floor enforced here (belt + suspenders after prompt)
        if e.confidence < 0.85 {
            return Err(TrustyError::BelowThreshold(e.name.clone(), e.confidence));
        }
        // Name sanity
        if e.name.trim().is_empty() || e.name.len() > 200 {
            return Err(TrustyError::ValidationError("Invalid entity name".into()));
        }
        Ok(())
    }

    fn validate_relationship(rel: &RawRelationship, entities: &[RawEntity]) -> TrustyResult<()> {
        // Both endpoints must exist in the entities list
        let names: HashSet<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        if !names.contains(rel.from_entity.as_str()) {
            return Err(TrustyError::ValidationError(
                format!("Relationship from unknown entity '{}'", rel.from_entity)
            ));
        }
        if !names.contains(rel.to_entity.as_str()) {
            return Err(TrustyError::ValidationError(
                format!("Relationship to unknown entity '{}'", rel.to_entity)
            ));
        }
        // Evidence required for all relationships
        if rel.evidence.trim().is_empty() {
            return Err(TrustyError::ValidationError(
                "Relationship missing evidence".into()
            ));
        }
        Ok(())
    }
}
```

---

### Stage 5: Confidence Filtering

Only entities with `confidence >= 0.85` pass. This is enforced in two places:
1. The LLM prompt instructs the model not to output below 0.85
2. `EntityParser::validate_entity` rejects anything below 0.85 regardless

This double enforcement handles cases where the LLM ignores instructions.

---

### Stage 6: Deduplication

Deduplication operates at two levels:

**Level 1: Fingerprint-based (fast path)**

```rust
pub fn fingerprint(name: &str, entity_type: EntityType) -> String {
    let normalized = normalize(name);
    let input = format!("{}:{}", entity_type.as_str(), normalized);
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(&hash[..16])  // 128-bit prefix, sufficient for dedup
}

pub fn normalize(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        // Remove common suffixes: "Inc.", "LLC", "Ltd.", "Corp."
        .replace(" inc.", "")
        .replace(" llc", "")
        .replace(" ltd.", "")
        .replace(" corp.", "")
        .replace(" corporation", "")
}
```

Fingerprints are stored in SQLite:
```sql
CREATE TABLE entity_fingerprints (
    fingerprint TEXT PRIMARY KEY,
    entity_id   TEXT NOT NULL,
    first_seen  TEXT NOT NULL,
    seen_count  INTEGER NOT NULL DEFAULT 1
);
```

If the fingerprint exists AND `seen_count < 2`, the entity is staged but not yet written to LanceDB or Kuzu. It must be seen in at least 2 different emails before graduating to permanent storage. This prevents one-off noise.

**Level 2: Fuzzy matching (slow path, on miss)**

If no exact fingerprint match, check `EntityStore` for near-matches:
```rust
// Normalized form column in LanceDB metadata
let existing = store.entities()
    .get_by_normalized_form(&normalize(&candidate.name))
    .await?;

if existing.is_none() {
    // Also check BM25 for typo variants: "Acme" vs "Acme Corp"
    let bm25_hits = store.bm25_search(&normalize(&candidate.name), 3).await?;
    for hit in bm25_hits {
        if levenshtein(&normalize(&candidate.name), &normalize(&hit.name)) <= 2 {
            // Merge into existing entity, update last_seen and seen_count
            return Ok(MergeDecision::MergeWith(hit.id));
        }
    }
}
```

---

### Stage 7: Storage

After validation and dedup, entities and relationships are written:

```rust
// In trusty-extractor, after pipeline passes
pub async fn persist_results(
    results: Vec<EntityExtractionResult>,
    store: &Store,
    embedding_engine: &EmbeddingEngine,
) -> TrustyResult<PersistReport> {
    let mut report = PersistReport::default();

    for result in results {
        for entity in result.entities {
            // 1. Compute embedding for entity content
            let content = entity.to_searchable_text();
            let embedding = embedding_engine.embed_one(&content).await?;

            // 2. Write to LanceDB (upsert by entity ID)
            store.entities().upsert_with_embedding(&entity, embedding).await?;

            // 3. Write node to Kuzu graph
            store.graph().upsert_node(&entity).await?;

            // 4. Update tantivy BM25 index
            store.entities().update_bm25_index(&entity).await?;

            report.entities_written += 1;
        }

        for relationship in result.relationships {
            // Upsert edge in Kuzu (update confidence + last_seen if exists)
            store.graph().upsert_edge(&relationship).await?;
            report.relationships_written += 1;
        }
    }

    // Commit tantivy index (batch commit after all writes)
    store.entities().commit_bm25().await?;

    Ok(report)
}
```

**Entity content for embedding:**
```rust
impl Entity {
    pub fn to_searchable_text(&self) -> String {
        match self.entity_type {
            EntityType::Person => format!(
                "{} {} {}",
                self.name,
                self.attributes.get("role").unwrap_or(&"".into()),
                self.attributes.get("company").unwrap_or(&"".into()),
            ),
            EntityType::Company => format!(
                "{} {} {}",
                self.name,
                self.attributes.get("industry").unwrap_or(&"".into()),
                self.attributes.get("domain").unwrap_or(&"".into()),
            ),
            EntityType::Project => format!(
                "{} {}",
                self.name,
                self.attributes.get("description").unwrap_or(&"".into()),
            ),
            _ => self.name.clone(),
        }
    }
}
```

---

### Stage 8: BM25 Index Update

The tantivy index is updated in the same transaction as LanceDB:

```rust
// Schema defined in trusty-embeddings
pub struct IndexDocument {
    pub entity_id: String,
    pub content:   String,   // result of to_searchable_text()
    pub entity_type: String,
    pub user_id:   String,
}
```

tantivy documents are committed in a batch after all emails in the sync cycle are processed, not per-entity. This avoids excessive merge overhead.

---

## Noise Reduction Rules

These rules are stricter than izzie2's extraction. Each rule has a rationale.

| Rule | Implementation | Rationale |
|------|---------------|-----------|
| Person from headers only | `source != "header"` → discard | Body mentions are unreliable; forwarded emails, signatures, and quotes create phantom persons |
| WORKS_WITH requires explicit collaboration evidence | Evidence string must reference a shared task/project | Co-mention (both on CC) is not evidence of collaboration |
| Max 3 relationships per email | Prompt instruction + parser enforcement | More relationships → lower average quality |
| Min 2 occurrences before storage | `seen_count >= 2` in fingerprint table | Prevents one-off contacts (cold outreach, receipts) from entering graph |
| Confidence threshold 0.85 | Prompt + parser | Stricter than izzie2's 0.7 |
| Newsletter/spam score < 0.3 | Stage 2 filter | Marketing emails generate fake entities (brands, promotions) |
| No relationships between CC-only participants | Prompt rule 6 | CC list co-occurrence is meaningless for graph quality |
| Strip quoted reply chains before extraction | Stage 1 body processing | Old quoted text creates duplicate and outdated entity mentions |

---

## Training vs Discovery Budget

**Discovery mode** (default): Process new emails from cursor. Budget per cycle: up to 50 emails, up to 200 extraction LLM calls (allows for spam filter calls). Runs every 30 minutes.

**Reprocessing** (triggered manually or after config change): Re-evaluate emails from a given date range. Uses a separate reprocessing cursor stored separately from the main cursor so it does not interfere with the live discovery cursor.

```sql
-- Reprocessing state
CREATE TABLE reprocessing_jobs (
    job_id      TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL,
    from_date   TEXT NOT NULL,
    to_date     TEXT NOT NULL,
    status      TEXT NOT NULL CHECK(status IN ('pending','running','done','failed')),
    progress    INTEGER DEFAULT 0,
    total       INTEGER DEFAULT 0,
    started_at  TEXT,
    completed_at TEXT,
    error       TEXT
);
```

**Budget guard:**
```rust
const MAX_DISCOVERY_EMAILS_PER_CYCLE: u32 = 50;
const MAX_LLM_CALLS_PER_CYCLE: u32 = 200; // includes spam filter calls
```

If the budget is exhausted mid-cycle, the cursor is updated to the last successfully processed email, and the next cycle picks up from there. No emails are lost.

---

## Deduplication Fingerprinting Detail

Fingerprint lifecycle:

```
Email arrives
     │
     ▼
Compute fingerprint for each extracted entity
     │
     ├── Fingerprint exists + seen_count >= 2?  → Upsert into LanceDB/Kuzu (update last_seen)
     │
     ├── Fingerprint exists + seen_count == 1?  → Increment seen_count, don't write to graph yet
     │
     └── Fingerprint new?
              │
              ├── Fuzzy match found in LanceDB?  → Treat as existing, merge metadata
              │
              └── No match?  → Insert fingerprint with seen_count=1
                              (will graduate to graph on second occurrence)
```

The 2-occurrence threshold means a contact you emailed once does not appear in your knowledge graph until you email them again. This is intentional — one-off emails (service receipts, cold outreach responses) should not pollute the graph.
