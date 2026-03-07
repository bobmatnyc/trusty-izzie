#!/usr/bin/env python3
"""
migrate_from_weaviate.py
========================
Migrates entities, relationships, and memories from izzie2's Weaviate Cloud
to trusty-izzie's local LanceDB + Kuzu store.

Re-embeds all content with all-MiniLM-L6-v2 (384-dim) since Weaviate used
text-embedding-ada-002 (1536-dim) — incompatible dimensions.

Issues detected are written to migration/issues.log for learning.

Usage:
    python3 migration/migrate_from_weaviate.py
"""

import os
import sys
import json
import math
import hashlib
import logging
from pathlib import Path
from datetime import datetime, timezone
from typing import Any
from collections import defaultdict

# ── deps ──────────────────────────────────────────────────────────────────────
try:
    import weaviate
    from weaviate.auth import Auth
    import lancedb
    import pyarrow as pa
    from sentence_transformers import SentenceTransformer
    import kuzu
except ImportError as e:
    print(f"Missing dependency: {e}")
    print("Run: /opt/homebrew/bin/pip3 install weaviate-client lancedb sentence-transformers kuzu pyarrow")
    sys.exit(1)

# ── config ────────────────────────────────────────────────────────────────────
WEAVIATE_URL = os.environ.get("WEAVIATE_URL", "")
WEAVIATE_API_KEY = os.environ.get("WEAVIATE_API_KEY", "")
DATA_DIR = Path(os.environ.get("TRUSTY_DATA_DIR", Path.home() / ".local/share/trusty-izzie")).expanduser()
EMBEDDING_MODEL = "all-MiniLM-L6-v2"
CONFIDENCE_THRESHOLD = 0.85   # Only migrate high-confidence entities
MIN_OCCURRENCES = 1            # Migration: allow 1 (can't know occurrence count from Weaviate)
VECTOR_DIM = 384

# Single-tenant design: trusty-izzie is dedicated to ONE user per instance.
# Instance ID = SHA256(primary_email)[:16]. Set PRIMARY_EMAIL to auto-derive.
# If not set, the script will use the first (and likely only) Weaviate tenant found.
PRIMARY_EMAIL = os.environ.get("TRUSTY_PRIMARY_EMAIL", "")

ENTITY_COLLECTIONS = ["Person", "Company", "Project", "Tool", "Topic", "Location", "ActionItem"]
RELATIONSHIP_COLLECTION = "Relationship"
MEMORY_COLLECTION = "Memory"
RESEARCH_COLLECTION = "ResearchFinding"

# ── logging ───────────────────────────────────────────────────────────────────
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[
        logging.StreamHandler(sys.stdout),
        logging.FileHandler("migration/migration.log"),
    ],
)
log = logging.getLogger(__name__)

issues: list[dict] = []

def record_issue(severity: str, collection: str, record_id: str, description: str, data: dict | None = None):
    issue = {
        "severity": severity,
        "collection": collection,
        "record_id": record_id,
        "description": description,
        "data": data or {},
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }
    issues.append(issue)
    level = logging.WARNING if severity == "warn" else logging.ERROR
    log.log(level, f"[{severity.upper()}] {collection}/{record_id}: {description}")


# ── LanceDB schema ────────────────────────────────────────────────────────────
def entity_schema() -> pa.Schema:
    return pa.schema([
        pa.field("id",              pa.string()),
        pa.field("user_id",         pa.string()),
        pa.field("entity_type",     pa.string()),
        pa.field("value",           pa.string()),
        pa.field("normalized",      pa.string()),
        pa.field("confidence",      pa.float32()),
        pa.field("source",          pa.string()),
        pa.field("source_id",       pa.string()),
        pa.field("context",         pa.string()),
        pa.field("aliases",         pa.string()),     # JSON array
        pa.field("occurrence_count",pa.int32()),
        pa.field("first_seen",      pa.string()),
        pa.field("last_seen",       pa.string()),
        pa.field("created_at",      pa.string()),
        pa.field("vector",          pa.list_(pa.float32(), VECTOR_DIM)),
    ])

def memory_schema() -> pa.Schema:
    return pa.schema([
        pa.field("id",              pa.string()),
        pa.field("user_id",         pa.string()),
        pa.field("content",         pa.string()),
        pa.field("category",        pa.string()),
        pa.field("source_type",     pa.string()),
        pa.field("source_id",       pa.string()),
        pa.field("importance",      pa.float32()),
        pa.field("decay_rate",      pa.float32()),
        pa.field("confidence",      pa.float32()),
        pa.field("last_accessed",   pa.string()),
        pa.field("expires_at",      pa.string()),
        pa.field("related_entities",pa.string()),     # JSON array
        pa.field("tags",            pa.string()),     # JSON array
        pa.field("created_at",      pa.string()),
        pa.field("is_deleted",      pa.bool_()),
        pa.field("vector",          pa.list_(pa.float32(), VECTOR_DIM)),
    ])


# ── Kuzu schema ───────────────────────────────────────────────────────────────
KUZU_SCHEMA_DDL = """
CREATE NODE TABLE IF NOT EXISTS Person(id STRING, user_id STRING, value STRING, normalized STRING, PRIMARY KEY(id));
CREATE NODE TABLE IF NOT EXISTS Company(id STRING, user_id STRING, value STRING, normalized STRING, PRIMARY KEY(id));
CREATE NODE TABLE IF NOT EXISTS Project(id STRING, user_id STRING, value STRING, normalized STRING, PRIMARY KEY(id));
CREATE NODE TABLE IF NOT EXISTS Tool(id STRING, user_id STRING, value STRING, normalized STRING, PRIMARY KEY(id));
CREATE NODE TABLE IF NOT EXISTS Topic(id STRING, user_id STRING, value STRING, normalized STRING, PRIMARY KEY(id));
CREATE NODE TABLE IF NOT EXISTS Location(id STRING, user_id STRING, value STRING, normalized STRING, PRIMARY KEY(id));
CREATE NODE TABLE IF NOT EXISTS ActionItem(id STRING, user_id STRING, value STRING, normalized STRING, PRIMARY KEY(id));

CREATE REL TABLE IF NOT EXISTS WORKS_FOR(FROM Person TO Company, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS WORKS_WITH(FROM Person TO Person, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS WORKS_ON(FROM Person TO Project, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS REPORTS_TO(FROM Person TO Person, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS LEADS(FROM Person TO Project, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS EXPERT_IN(FROM Person TO Topic, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS LOCATED_IN(FROM Person TO Location, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS PARTNERS_WITH(FROM Company TO Company, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
CREATE REL TABLE IF NOT EXISTS RELATED_TO(FROM Topic TO Topic, confidence FLOAT, evidence STRING, source_id STRING, status STRING, first_seen STRING, last_seen STRING);
"""

REL_TYPE_MAP = {
    "WORKS_FOR":     "WORKS_FOR",
    "WORKS_WITH":    "WORKS_WITH",
    "WORKS_ON":      "WORKS_ON",
    "REPORTS_TO":    "REPORTS_TO",
    "LEADS":         "LEADS",
    "EXPERT_IN":     "EXPERT_IN",
    "LOCATED_IN":    "LOCATED_IN",
    "PARTNERS_WITH": "PARTNERS_WITH",
    "RELATED_TO":    "RELATED_TO",
    "DEPENDS_ON":    "RELATED_TO",
    "PART_OF":       "RELATED_TO",
    "FRIEND_OF":     "WORKS_WITH",
    "FAMILY_OF":     "WORKS_WITH",
    "MARRIED_TO":    "WORKS_WITH",
    "SIBLING_OF":    "WORKS_WITH",
    "ASSOCIATED_WITH": "RELATED_TO",
    "COMPETES_WITH": "RELATED_TO",
    "OWNS":          "RELATED_TO",
}

# ── helpers ───────────────────────────────────────────────────────────────────
def safe_str(v: Any, default: str = "") -> str:
    if v is None:
        return default
    return str(v).strip()

def safe_float(v: Any, default: float = 0.0) -> float:
    try:
        return float(v)
    except (TypeError, ValueError):
        return default

def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()

def instance_id_from_email(email: str) -> str:
    """Derive stable per-instance user ID from primary email (SHA256 hex, first 16 chars)."""
    return hashlib.sha256(email.lower().strip().encode()).hexdigest()[:16]

def stable_id(entity_type: str, value: str, user_id: str) -> str:
    """Deterministic UUID-like ID from type+value+user for deduplication."""
    h = hashlib.sha256(f"{entity_type}:{value.lower()}:{user_id}".encode()).hexdigest()
    return f"{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:32]}"

def normalize(value: str) -> str:
    import re
    return re.sub(r"[^a-z0-9]+", "_", value.lower()).strip("_")

def decay_rate_for_category(category: str) -> float:
    rates = {
        "preference": 0.01, "fact": 0.02, "relationship": 0.02,
        "decision": 0.03, "event": 0.05, "sentiment": 0.1, "reminder": 0.2,
    }
    return rates.get(category, 0.03)


# ── embedding ─────────────────────────────────────────────────────────────────
def load_embedder():
    log.info(f"Loading embedding model: {EMBEDDING_MODEL}")
    model = SentenceTransformer(EMBEDDING_MODEL)
    log.info("Embedding model loaded")
    return model

def embed_batch(model: SentenceTransformer, texts: list[str]) -> list[list[float]]:
    if not texts:
        return []
    vecs = model.encode(texts, batch_size=64, show_progress_bar=False, normalize_embeddings=True)
    return vecs.tolist()


# ── Weaviate fetch ────────────────────────────────────────────────────────────
def fetch_tenants(client: weaviate.WeaviateClient, collection_name: str) -> list[str]:
    try:
        col = client.collections.get(collection_name)
        tenants = col.tenants.get()
        return list(tenants.keys())
    except Exception as e:
        log.warning(f"Could not list tenants for {collection_name}: {e}")
        return []

def fetch_all_objects(client: weaviate.WeaviateClient, collection_name: str, tenant_id: str, limit: int = 1000) -> list[dict]:
    """Fetch all objects from a collection for a given tenant."""
    try:
        col = client.collections.get(collection_name).with_tenant(tenant_id)
        objects = []
        response = col.query.fetch_objects(limit=limit, include_vector=False)
        for obj in response.objects:
            props = dict(obj.properties)
            props["_uuid"] = str(obj.uuid)
            objects.append(props)

        # Paginate if needed
        while len(response.objects) == limit:
            last_uuid = str(response.objects[-1].uuid)
            response = col.query.fetch_objects(limit=limit, after=last_uuid, include_vector=False)
            for obj in response.objects:
                props = dict(obj.properties)
                props["_uuid"] = str(obj.uuid)
                objects.append(props)

        log.info(f"  Fetched {len(objects)} objects from {collection_name} (tenant={tenant_id})")
        return objects
    except Exception as e:
        log.error(f"  Failed to fetch {collection_name}/{tenant_id}: {e}")
        record_issue("error", collection_name, tenant_id, f"Fetch failed: {e}")
        return []


# ── entity validation ─────────────────────────────────────────────────────────
def validate_entity(obj: dict, collection: str) -> tuple[bool, list[str]]:
    """Returns (is_valid, list_of_issues)."""
    problems = []

    value = safe_str(obj.get("value"))
    confidence = safe_float(obj.get("confidence"), 0.0)

    if not value:
        problems.append("missing value field")
    if len(value) < 2:
        problems.append(f"suspiciously short value: '{value}'")
    if confidence < 0.5:
        problems.append(f"very low confidence: {confidence:.2f}")
    if confidence < CONFIDENCE_THRESHOLD:
        problems.append(f"below migration threshold ({confidence:.2f} < {CONFIDENCE_THRESHOLD})")

    # Person-specific: should come from headers
    if collection == "Person":
        source = safe_str(obj.get("source", ""))
        if source not in ("header", "metadata", ""):
            problems.append(f"person from non-header source: '{source}'")

    return len([p for p in problems if "below migration threshold" in p or "missing" in p]) == 0, problems


# ── main migration ─────────────────────────────────────────────────────────────
def main():
    log.info("=" * 60)
    log.info("trusty-izzie: Weaviate → LanceDB + Kuzu migration")
    log.info("=" * 60)

    # Setup data directory
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    lance_path = DATA_DIR / "lance"
    kuzu_path  = DATA_DIR / "kuzu"
    lance_path.mkdir(exist_ok=True)

    # ── Connect to Weaviate ──────────────────────────────────────────────────
    log.info(f"Connecting to Weaviate: {WEAVIATE_URL}")
    wv_client = weaviate.connect_to_weaviate_cloud(
        cluster_url=WEAVIATE_URL,
        auth_credentials=Auth.api_key(WEAVIATE_API_KEY),
    )
    log.info("Connected to Weaviate")

    # ── Discover tenant IDs ──────────────────────────────────────────────────
    log.info("Discovering tenant IDs from Person collection...")
    tenants = fetch_tenants(wv_client, "Person")
    log.info(f"Found {len(tenants)} tenant(s): {tenants}")

    if not tenants:
        log.error("No tenants found — nothing to migrate")
        wv_client.close()
        sys.exit(1)

    # Single-tenant design: pick the right Weaviate tenant.
    # If PRIMARY_EMAIL is set, derive instance_id from it and find matching tenant.
    # Otherwise use the only tenant (or first if multiple).
    if PRIMARY_EMAIL:
        instance_id = instance_id_from_email(PRIMARY_EMAIL)
        log.info(f"Primary email set — instance ID: {instance_id}")
        # The Weaviate tenant key is the izzie2 userId (UUID from better-auth).
        # We can't directly match by email, so we migrate the first tenant's data
        # but store it under our derived instance ID.
        weaviate_tenant = tenants[0]
        if len(tenants) > 1:
            log.warning(f"Multiple tenants found: {tenants}. Migrating: {weaviate_tenant}")
    else:
        weaviate_tenant = tenants[0]
        instance_id = weaviate_tenant[:16]
        log.info(f"No PRIMARY_EMAIL set — using Weaviate tenant as instance ID: {instance_id}")

    log.info(f"Instance ID (stored in local DB): {instance_id}")
    log.info(f"Weaviate tenant to migrate:       {weaviate_tenant}")

    # Write instance ID to a local file for trusty-izzie to read on startup
    instance_file = DATA_DIR / "instance.json"
    with open(instance_file, "w") as f:
        json.dump({
            "instance_id": instance_id,
            "primary_email": PRIMARY_EMAIL,
            "weaviate_tenant_migrated_from": weaviate_tenant,
            "migrated_at": now_iso(),
        }, f, indent=2)
    log.info(f"Instance metadata written to: {instance_file}")

    # ── Load embedding model ─────────────────────────────────────────────────
    embedder = load_embedder()

    # ── Connect to LanceDB ───────────────────────────────────────────────────
    log.info(f"Opening LanceDB at: {lance_path}")
    db = lancedb.connect(str(lance_path))

    # ── Connect to Kuzu ──────────────────────────────────────────────────────
    log.info(f"Opening Kuzu at: {kuzu_path}")
    kz_db   = kuzu.Database(str(kuzu_path))
    kz_conn = kuzu.Connection(kz_db)

    # Create Kuzu schema
    log.info("Initializing Kuzu schema...")
    for stmt in KUZU_SCHEMA_DDL.strip().split(";"):
        stmt = stmt.strip()
        if stmt:
            try:
                kz_conn.execute(stmt)
            except Exception as e:
                if "already exists" not in str(e).lower():
                    log.warning(f"Kuzu DDL warning: {e}")

    # ── Stats ────────────────────────────────────────────────────────────────
    stats = defaultdict(lambda: {"fetched": 0, "migrated": 0, "skipped": 0, "issues": 0})

    # ── Single tenant migration ───────────────────────────────────────────────
    # trusty-izzie is single-tenant: one instance per user.
    # LanceDB tables are simply "entities" and "memories" (no user prefix).
    tenant_id = weaviate_tenant   # which tenant to read from Weaviate
    user_id   = instance_id       # local instance identifier

    for _once in [1]:
        log.info(f"\n{'─'*50}")
        log.info(f"Processing tenant: {tenant_id} → instance: {user_id}")
        log.info(f"{'─'*50}")

        kuzu_nodes: dict[str, dict] = {}   # normalized_key → {id, type, value}

        # ── Entities ─────────────────────────────────────────────────────────
        entity_rows: list[dict] = []
        entity_texts: list[str] = []

        for coll in ENTITY_COLLECTIONS:
            objects = fetch_all_objects(wv_client, coll, tenant_id)
            stats[coll]["fetched"] += len(objects)

            for obj in objects:
                value      = safe_str(obj.get("value"))
                confidence = safe_float(obj.get("confidence"), 0.0)
                uuid       = safe_str(obj.get("_uuid"))

                if not value:
                    record_issue("error", coll, uuid, "Empty value — skipping")
                    stats[coll]["skipped"] += 1
                    continue

                is_valid, problems = validate_entity(obj, coll)

                for p in problems:
                    severity = "error" if not is_valid else "warn"
                    record_issue(severity, coll, uuid, p, {"value": value, "confidence": confidence})
                    stats[coll]["issues"] += 1

                if not is_valid:
                    stats[coll]["skipped"] += 1
                    continue

                norm = normalize(value)
                entity_id = stable_id(coll.lower(), value, tenant_id)
                now = now_iso()
                extracted_at = safe_str(obj.get("extractedAt", now))

                row = {
                    "id":               entity_id,
                    "user_id":          user_id,
                    "entity_type":      coll.lower(),
                    "value":            value,
                    "normalized":       norm,
                    "confidence":       float(confidence),
                    "source":           safe_str(obj.get("source", "unknown")),
                    "source_id":        safe_str(obj.get("sourceId", "")),
                    "context":          safe_str(obj.get("context", "")),
                    "aliases":          json.dumps(obj.get("aliases") or []),
                    "occurrence_count": 1,
                    "first_seen":       extracted_at,
                    "last_seen":        extracted_at,
                    "created_at":       now,
                }
                entity_rows.append(row)
                entity_texts.append(f"{coll}: {value}. {safe_str(obj.get('context', ''))}")

                # Track for Kuzu graph
                kuzu_nodes[f"{coll}::{norm}"] = {
                    "id": entity_id, "type": coll, "value": value, "normalized": norm,
                }

        # Embed entities in batch
        log.info(f"Embedding {len(entity_texts)} entities...")
        entity_vectors = embed_batch(embedder, entity_texts) if entity_texts else []
        for row, vec in zip(entity_rows, entity_vectors):
            row["vector"] = vec

        # Write to LanceDB
        if entity_rows:
            log.info(f"Writing {len(entity_rows)} entities to LanceDB...")
            tbl_name = "entities"   # single-tenant: no prefix needed
            try:
                if tbl_name in db.table_names():
                    tbl = db.open_table(tbl_name)
                    tbl.add(entity_rows, mode="overwrite")
                else:
                    db.create_table(tbl_name, entity_rows, schema=entity_schema())
                log.info(f"  ✓ {len(entity_rows)} entities written to LanceDB table '{tbl_name}'")
            except Exception as e:
                log.error(f"  ✗ LanceDB entity write failed: {e}")
                record_issue("error", "LanceDB", tbl_name, f"Write failed: {e}")

        for coll in ENTITY_COLLECTIONS:
            stats[coll]["migrated"] = sum(1 for r in entity_rows if r["entity_type"] == coll.lower())

        # Write entity nodes to Kuzu
        log.info(f"Writing {len(kuzu_nodes)} entity nodes to Kuzu...")
        kuzu_written = 0
        for key, node in kuzu_nodes.items():
            ntype = node["type"]
            try:
                # Kuzu 0.9: no MERGE — check existence then create
                check = kz_conn.execute(
                    f'MATCH (n:{ntype} {{id: $id}}) RETURN count(n) AS cnt',
                    {"id": node["id"]}
                )
                exists = check.get_next()[0] > 0 if check.has_next() else False
                if not exists:
                    kz_conn.execute(
                        f'CREATE (:{ntype} {{id: $id, user_id: $user_id, value: $value, normalized: $normalized}})',
                        {"id": node["id"], "user_id": user_id,
                         "value": node["value"], "normalized": node["normalized"]}
                    )
                kuzu_written += 1
            except Exception as e:
                if "already exists" not in str(e).lower() and "duplicate" not in str(e).lower():
                    record_issue("warn", f"Kuzu/{ntype}", node["id"], f"Node insert failed: {e}")
        log.info(f"  ✓ {kuzu_written} Kuzu nodes written")

        # ── Relationships ─────────────────────────────────────────────────────
        log.info("\nFetching relationships...")
        rel_objects = fetch_all_objects(wv_client, RELATIONSHIP_COLLECTION, tenant_id)
        stats["Relationship"]["fetched"] += len(rel_objects)

        rel_written = 0
        for obj in rel_objects:
            from_type  = safe_str(obj.get("fromEntityType", "")).capitalize()
            from_val   = safe_str(obj.get("fromEntityValue", ""))
            to_type    = safe_str(obj.get("toEntityType", "")).capitalize()
            to_val     = safe_str(obj.get("toEntityValue", ""))
            rel_type   = safe_str(obj.get("relationshipType", "")).upper()
            confidence = safe_float(obj.get("confidence"), 0.0)
            uuid       = safe_str(obj.get("_uuid"))

            if not all([from_type, from_val, to_type, to_val, rel_type]):
                record_issue("warn", "Relationship", uuid, "Incomplete relationship fields — skipping",
                             {"from": f"{from_type}:{from_val}", "to": f"{to_type}:{to_val}", "type": rel_type})
                stats["Relationship"]["skipped"] += 1
                continue

            if confidence < 0.7:
                record_issue("warn", "Relationship", uuid,
                             f"Low-confidence relationship ({confidence:.2f}) — skipping",
                             {"from": from_val, "to": to_val, "type": rel_type})
                stats["Relationship"]["skipped"] += 1
                continue

            mapped_rel = REL_TYPE_MAP.get(rel_type)
            if not mapped_rel:
                record_issue("warn", "Relationship", uuid, f"Unknown relationship type '{rel_type}' — mapping to RELATED_TO")
                mapped_rel = "RELATED_TO"
                stats["Relationship"]["issues"] += 1

            from_id = kuzu_nodes.get(f"{from_type}::{normalize(from_val)}", {}).get("id")
            to_id   = kuzu_nodes.get(f"{to_type}::{normalize(to_val)}", {}).get("id")

            if not from_id or not to_id:
                record_issue("warn", "Relationship", uuid,
                             f"Entity not in migrated set: {from_type}:{from_val} → {to_type}:{to_val}")
                stats["Relationship"]["skipped"] += 1
                continue

            extracted_at = safe_str(obj.get("inferredAt", now_iso()))
            try:
                kz_conn.execute(
                    f'MATCH (a:{from_type} {{id: $from_id}}), (b:{to_type} {{id: $to_id}}) '
                    f'CREATE (a)-[:{mapped_rel} {{confidence: $conf, evidence: $ev, source_id: $sid, '
                    f'status: $status, first_seen: $fs, last_seen: $ls}}]->(b)',
                    {
                        "from_id": from_id,
                        "to_id":   to_id,
                        "conf":    float(confidence),
                        "ev":      safe_str(obj.get("evidence", "")),
                        "sid":     safe_str(obj.get("sourceId", "")),
                        "status":  safe_str(obj.get("status", "active")),
                        "fs":      extracted_at,
                        "ls":      extracted_at,
                    }
                )
                rel_written += 1
                stats["Relationship"]["migrated"] += 1
            except Exception as e:
                record_issue("warn", "Relationship", uuid, f"Kuzu edge insert failed: {e}")
                stats["Relationship"]["issues"] += 1

        log.info(f"  ✓ {rel_written}/{len(rel_objects)} relationships written to Kuzu")

        # ── Memories ──────────────────────────────────────────────────────────
        log.info("\nFetching memories...")
        mem_objects = fetch_all_objects(wv_client, MEMORY_COLLECTION, tenant_id)
        stats["Memory"]["fetched"] += len(mem_objects)

        memory_rows: list[dict] = []
        memory_texts: list[str] = []

        for obj in mem_objects:
            content    = safe_str(obj.get("content", ""))
            category   = safe_str(obj.get("category", "fact"))
            importance = safe_float(obj.get("importance", 0.5))
            confidence = safe_float(obj.get("confidence", 0.8))
            is_deleted = bool(obj.get("isDeleted", False))
            uuid       = safe_str(obj.get("_uuid"))

            if not content:
                record_issue("warn", "Memory", uuid, "Empty content — skipping")
                stats["Memory"]["skipped"] += 1
                continue

            if is_deleted:
                stats["Memory"]["skipped"] += 1
                continue

            if importance < 0.3:
                record_issue("warn", "Memory", uuid, f"Very low importance ({importance:.2f}) — skipping")
                stats["Memory"]["skipped"] += 1
                continue

            now = now_iso()
            row = {
                "id":               stable_id("memory", content[:64], tenant_id),
                "user_id":          user_id,
                "content":          content,
                "category":         category,
                "source_type":      safe_str(obj.get("sourceType", "chat")),
                "source_id":        safe_str(obj.get("sourceId", "")),
                "importance":       float(importance),
                "decay_rate":       float(obj.get("decayRate") or decay_rate_for_category(category)),
                "confidence":       float(confidence),
                "last_accessed":    safe_str(obj.get("lastAccessed", now)),
                "expires_at":       safe_str(obj.get("expiresAt", "")),
                "related_entities": json.dumps(obj.get("relatedEntities") or []),
                "tags":             json.dumps(obj.get("tags") or []),
                "created_at":       safe_str(obj.get("createdAt", now)),
                "is_deleted":       False,
            }
            memory_rows.append(row)
            memory_texts.append(f"[{category}] {content}")

        # Embed memories
        log.info(f"Embedding {len(memory_texts)} memories...")
        memory_vectors = embed_batch(embedder, memory_texts) if memory_texts else []
        for row, vec in zip(memory_rows, memory_vectors):
            row["vector"] = vec

        # Write memories to LanceDB
        if memory_rows:
            log.info(f"Writing {len(memory_rows)} memories to LanceDB...")
            tbl_name = "memories"   # single-tenant: no prefix needed
            try:
                if tbl_name in db.table_names():
                    tbl = db.open_table(tbl_name)
                    tbl.add(memory_rows, mode="overwrite")
                else:
                    db.create_table(tbl_name, memory_rows, schema=memory_schema())
                log.info(f"  ✓ {len(memory_rows)} memories written to LanceDB table '{tbl_name}'")
                stats["Memory"]["migrated"] += len(memory_rows)
            except Exception as e:
                log.error(f"  ✗ LanceDB memory write failed: {e}")
                record_issue("error", "Memory", tbl_name, f"Write failed: {e}")

    # ── Close connections ─────────────────────────────────────────────────────
    wv_client.close()
    log.info("\nWeaviate connection closed")

    # ── Write issues report ───────────────────────────────────────────────────
    issues_path = Path("migration/issues.json")
    with open(issues_path, "w") as f:
        json.dump(issues, f, indent=2)
    log.info(f"Issues written to: {issues_path}")

    # ── Summary ───────────────────────────────────────────────────────────────
    log.info("\n" + "=" * 60)
    log.info("MIGRATION SUMMARY")
    log.info("=" * 60)
    total_migrated = 0
    total_skipped  = 0
    total_issues   = 0
    for coll, s in sorted(stats.items()):
        log.info(f"  {coll:20s}  fetched={s['fetched']:4d}  migrated={s['migrated']:4d}  "
                 f"skipped={s['skipped']:4d}  issues={s['issues']:4d}")
        total_migrated += s["migrated"]
        total_skipped  += s["skipped"]
        total_issues   += s["issues"]

    log.info("─" * 60)
    log.info(f"  {'TOTAL':20s}  migrated={total_migrated:4d}  skipped={total_skipped:4d}  issues={total_issues:4d}")
    log.info("=" * 60)

    # Entity type breakdown
    log.info("\nISSUE BREAKDOWN:")
    by_desc = defaultdict(int)
    for iss in issues:
        key = iss["description"].split(":")[0].split("(")[0].strip()
        by_desc[key] += 1
    for desc, count in sorted(by_desc.items(), key=lambda x: -x[1])[:15]:
        log.info(f"  {count:4d}x  {desc}")

    log.info(f"\n✓ Migration complete. Data at: {DATA_DIR}")
    log.info(f"  LanceDB tables: {db.table_names()}")


if __name__ == "__main__":
    main()
