//! Kuzu graph database store for entities and relationships.

use anyhow::Result;
use std::path::Path;
use tracing::debug;

use trusty_models::entity::{
    Entity, EntityType, Relationship, RelationshipStatus, RelationshipType,
};

/// Handle to the Kuzu embedded graph database.
pub struct GraphStore {
    db: kuzu::Database,
}

impl GraphStore {
    /// Open (or create) a Kuzu database at `path` in read-write mode.
    ///
    /// Kuzu creates the database directory itself; we only ensure the *parent*
    /// directory exists. Creating the target directory upfront causes EEXIST
    /// failures when a file (e.g. from a Python-created single-file DB) already
    /// occupies that path.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_mode(path, false)
    }

    /// Open an existing Kuzu database at `path` in read-only mode.
    ///
    /// Read-only mode allows multiple processes to open the database
    /// concurrently without lock contention. Only the daemon should open
    /// in read-write mode.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        Self::open_with_mode(path, true)
    }

    /// Open a Kuzu database at `path` with the specified read-only mode.
    pub(crate) fn open_with_mode(path: &Path, read_only: bool) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let config = kuzu::SystemConfig::default().read_only(read_only);
        let db = kuzu::Database::new(path, config)?;
        let store = Self { db };
        if !read_only {
            store.migrate()?;
        }
        Ok(store)
    }

    /// Ensure the schema (node and edge tables) exists. Safe to call multiple times.
    pub fn migrate(&self) -> Result<()> {
        let conn = kuzu::Connection::new(&self.db)?;

        // Each DDL is run individually; "already exists" errors are swallowed.
        let ddl_statements = [
            // Node tables
            "CREATE NODE TABLE Person (id STRING, name STRING, email STRING, role STRING, confidence DOUBLE, first_seen STRING, last_seen STRING, PRIMARY KEY (id));",
            "CREATE NODE TABLE Company (id STRING, name STRING, domain STRING, industry STRING, confidence DOUBLE, first_seen STRING, last_seen STRING, PRIMARY KEY (id));",
            "CREATE NODE TABLE Project (id STRING, name STRING, status STRING, description STRING, confidence DOUBLE, first_seen STRING, last_seen STRING, PRIMARY KEY (id));",
            "CREATE NODE TABLE Tool (id STRING, name STRING, category STRING, url STRING, confidence DOUBLE, first_seen STRING, last_seen STRING, PRIMARY KEY (id));",
            "CREATE NODE TABLE Topic (id STRING, name STRING, description STRING, confidence DOUBLE, first_seen STRING, last_seen STRING, PRIMARY KEY (id));",
            "CREATE NODE TABLE Location (id STRING, name STRING, city STRING, country STRING, confidence DOUBLE, first_seen STRING, last_seen STRING, PRIMARY KEY (id));",
            // Edge tables
            "CREATE REL TABLE WORKS_FOR (FROM Person TO Company, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING, status STRING);",
            "CREATE REL TABLE WORKS_WITH (FROM Person TO Person, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
            "CREATE REL TABLE WORKS_ON (FROM Person TO Project, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
            "CREATE REL TABLE REPORTS_TO (FROM Person TO Person, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
            "CREATE REL TABLE LEADS (FROM Person TO Project, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
            "CREATE REL TABLE EXPERT_IN (FROM Person TO Topic, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
            "CREATE REL TABLE LOCATED_IN (FROM Person TO Location, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
            "CREATE REL TABLE PARTNERS_WITH (FROM Company TO Company, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
            "CREATE REL TABLE RELATED_TO (FROM Person TO Topic, confidence DOUBLE, evidence STRING, first_seen STRING, last_seen STRING);",
        ];

        for stmt in &ddl_statements {
            if let Err(e) = conn.query(stmt) {
                // Kuzu returns an error if the table already exists; we ignore it.
                let msg = e.to_string();
                if !msg.contains("already exists") && !msg.contains("Table") {
                    return Err(anyhow::anyhow!("Kuzu DDL failed: {}: {}", stmt, msg));
                }
                debug!("Kuzu DDL skipped (already exists): {}", stmt);
            }
        }
        Ok(())
    }

    /// Upsert an entity node. Routes to the correct table based on entity_type.
    pub fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        let conn = kuzu::Connection::new(&self.db)?;
        let id = entity.id.to_string();
        let name = escape_str(&entity.value);
        let confidence = entity.confidence as f64;
        let first_seen = entity.first_seen.to_rfc3339();
        let last_seen = entity.last_seen.to_rfc3339();

        match entity.entity_type {
            EntityType::Person => {
                // Delete existing, then create
                let _ = conn.query(&format!("MATCH (n:Person {{id: '{id}'}}) DELETE n;"));
                conn.query(&format!(
                    "CREATE (:Person {{id: '{id}', name: '{name}', email: '', role: '', confidence: {confidence}, first_seen: '{first_seen}', last_seen: '{last_seen}'}});"
                ))?;
            }
            EntityType::Company => {
                let _ = conn.query(&format!("MATCH (n:Company {{id: '{id}'}}) DELETE n;"));
                conn.query(&format!(
                    "CREATE (:Company {{id: '{id}', name: '{name}', domain: '', industry: '', confidence: {confidence}, first_seen: '{first_seen}', last_seen: '{last_seen}'}});"
                ))?;
            }
            EntityType::Project => {
                let _ = conn.query(&format!("MATCH (n:Project {{id: '{id}'}}) DELETE n;"));
                conn.query(&format!(
                    "CREATE (:Project {{id: '{id}', name: '{name}', status: '', description: '', confidence: {confidence}, first_seen: '{first_seen}', last_seen: '{last_seen}'}});"
                ))?;
            }
            EntityType::Tool => {
                let _ = conn.query(&format!("MATCH (n:Tool {{id: '{id}'}}) DELETE n;"));
                conn.query(&format!(
                    "CREATE (:Tool {{id: '{id}', name: '{name}', category: '', url: '', confidence: {confidence}, first_seen: '{first_seen}', last_seen: '{last_seen}'}});"
                ))?;
            }
            EntityType::Topic => {
                let _ = conn.query(&format!("MATCH (n:Topic {{id: '{id}'}}) DELETE n;"));
                conn.query(&format!(
                    "CREATE (:Topic {{id: '{id}', name: '{name}', description: '', confidence: {confidence}, first_seen: '{first_seen}', last_seen: '{last_seen}'}});"
                ))?;
            }
            EntityType::Location => {
                let _ = conn.query(&format!("MATCH (n:Location {{id: '{id}'}}) DELETE n;"));
                conn.query(&format!(
                    "CREATE (:Location {{id: '{id}', name: '{name}', city: '', country: '', confidence: {confidence}, first_seen: '{first_seen}', last_seen: '{last_seen}'}});"
                ))?;
            }
            EntityType::ActionItem => {
                // ActionItem has no dedicated node table; store as Topic
                let _ = conn.query(&format!("MATCH (n:Topic {{id: '{id}'}}) DELETE n;"));
                conn.query(&format!(
                    "CREATE (:Topic {{id: '{id}', name: '{name}', description: 'ActionItem', confidence: {confidence}, first_seen: '{first_seen}', last_seen: '{last_seen}'}});"
                ))?;
            }
        }
        Ok(())
    }

    /// Upsert a relationship edge.
    pub fn upsert_relationship(&self, rel: &Relationship) -> Result<()> {
        let conn = kuzu::Connection::new(&self.db)?;

        let from_id = &rel.from_entity_value;
        let to_id = &rel.to_entity_value;
        let confidence = rel.confidence as f64;
        let evidence = escape_str(rel.evidence.as_deref().unwrap_or(""));
        let first_seen = rel.first_seen.to_rfc3339();
        let last_seen = rel.last_seen.to_rfc3339();
        let status = match rel.status {
            RelationshipStatus::Active => "active",
            RelationshipStatus::Former => "former",
            RelationshipStatus::Unknown => "unknown",
        };

        // Determine which Cypher rel table and node types to use
        let (rel_table, from_label, to_label) = rel_table_for(&rel.relationship_type);

        // Skip unsupported relationship types (no matching table)
        if rel_table.is_empty() {
            return Ok(());
        }

        let cypher = format!(
            "MATCH (a:{from_label} {{id: '{from_id}'}}), (b:{to_label} {{id: '{to_id}'}}) \
             CREATE (a)-[:{rel_table} {{confidence: {confidence}, evidence: '{evidence}', first_seen: '{first_seen}', last_seen: '{last_seen}', status: '{status}'}}]->(b);"
        );
        // Ignore errors (e.g. nodes don't exist yet)
        let _ = conn.query(&cypher);
        Ok(())
    }

    /// Search for entities by name prefix (case-insensitive substring).
    pub fn search_entities(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        let conn = kuzu::Connection::new(&self.db)?;
        let q = escape_str(query).to_lowercase();
        let mut entities = Vec::new();

        for label in &["Person", "Company", "Project", "Tool", "Topic", "Location"] {
            let cypher = format!(
                "MATCH (n:{label}) WHERE lower(n.name) CONTAINS '{q}' RETURN n.id, n.name, n.confidence, n.first_seen, n.last_seen LIMIT {limit};"
            );
            let mut result = conn.query(&cypher)?;
            for row in &mut result {
                if entities.len() >= limit {
                    break;
                }
                let id_str = value_to_string(&row[0]);
                let id = match uuid::Uuid::parse_str(&id_str) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                let name = value_to_string(&row[1]);
                let confidence = value_to_f64(&row[2]) as f32;
                let first_seen = parse_dt(value_to_string(&row[3]).as_str());
                let last_seen = parse_dt(value_to_string(&row[4]).as_str());
                let entity_type = entity_type_from_label(label);

                entities.push(Entity {
                    id,
                    user_id: String::new(),
                    entity_type,
                    value: name.clone(),
                    normalized: name.to_lowercase(),
                    confidence,
                    source: "graph".to_string(),
                    source_id: None,
                    context: None,
                    aliases: vec![],
                    occurrence_count: 1,
                    first_seen,
                    last_seen,
                    created_at: first_seen,
                });
            }
            if entities.len() >= limit {
                break;
            }
        }
        Ok(entities)
    }

    /// List entities, optionally filtered by type string (e.g. "Person", "Company").
    /// Returns up to `limit` entities ordered by label then name.
    pub fn list_entities(
        &self,
        entity_type_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Entity>> {
        let conn = kuzu::Connection::new(&self.db)?;
        let mut entities = Vec::new();

        let labels: Vec<&str> = match entity_type_filter {
            Some(t) => vec![t],
            None => vec!["Person", "Company", "Project", "Tool", "Topic", "Location"],
        };

        for label in &labels {
            if entities.len() >= limit {
                break;
            }
            let remaining = limit - entities.len();
            let cypher = format!(
                "MATCH (n:{label}) RETURN n.id, n.name, n.confidence, n.first_seen, n.last_seen ORDER BY n.name LIMIT {remaining};"
            );
            let mut result = match conn.query(&cypher) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for row in &mut result {
                if entities.len() >= limit {
                    break;
                }
                let id_str = value_to_string(&row[0]);
                let id = match uuid::Uuid::parse_str(&id_str) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                let name = value_to_string(&row[1]);
                let confidence = value_to_f64(&row[2]) as f32;
                let first_seen = parse_dt(value_to_string(&row[3]).as_str());
                let last_seen = parse_dt(value_to_string(&row[4]).as_str());
                let entity_type = entity_type_from_label(label);

                entities.push(Entity {
                    id,
                    user_id: String::new(),
                    entity_type,
                    value: name.clone(),
                    normalized: name.to_lowercase(),
                    confidence,
                    source: "graph".to_string(),
                    source_id: None,
                    context: None,
                    aliases: vec![],
                    occurrence_count: 1,
                    first_seen,
                    last_seen,
                    created_at: first_seen,
                });
            }
        }
        Ok(entities)
    }

    /// Get all outgoing relationships for an entity (matched by id across all node types).
    pub fn get_entity_relationships(&self, entity_id: &str) -> Result<Vec<Relationship>> {
        let conn = kuzu::Connection::new(&self.db)?;
        let mut rels = Vec::new();

        for (rel_table, from_label, to_label) in ALL_REL_TABLES {
            let cypher = format!(
                "MATCH (a:{from_label} {{id: '{entity_id}'}})-[r:{rel_table}]->(b:{to_label}) \
                 RETURN b.id, r.confidence, r.evidence, r.first_seen, r.last_seen LIMIT 50;"
            );
            let mut result = match conn.query(&cypher) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for row in &mut result {
                let to_id = value_to_string(&row[0]);
                let confidence = value_to_f64(&row[1]) as f32;
                let evidence = value_to_string(&row[2]);
                let first_seen = parse_dt(value_to_string(&row[3]).as_str());
                let last_seen = parse_dt(value_to_string(&row[4]).as_str());

                rels.push(Relationship {
                    id: uuid::Uuid::new_v4(),
                    user_id: String::new(),
                    from_entity_type: entity_type_from_label(from_label),
                    from_entity_value: entity_id.to_string(),
                    to_entity_type: entity_type_from_label(to_label),
                    to_entity_value: to_id,
                    relationship_type: rel_type_from_table(rel_table),
                    confidence,
                    evidence: if evidence.is_empty() {
                        None
                    } else {
                        Some(evidence)
                    },
                    source_id: None,
                    status: RelationshipStatus::Unknown,
                    first_seen,
                    last_seen,
                });
            }
        }
        Ok(rels)
    }
}

// --- relationship table routing ---

const ALL_REL_TABLES: &[(&str, &str, &str)] = &[
    ("WORKS_FOR", "Person", "Company"),
    ("WORKS_WITH", "Person", "Person"),
    ("WORKS_ON", "Person", "Project"),
    ("REPORTS_TO", "Person", "Person"),
    ("LEADS", "Person", "Project"),
    ("EXPERT_IN", "Person", "Topic"),
    ("LOCATED_IN", "Person", "Location"),
    ("PARTNERS_WITH", "Company", "Company"),
    ("RELATED_TO", "Person", "Topic"),
];

fn rel_table_for(rt: &RelationshipType) -> (&'static str, &'static str, &'static str) {
    match rt {
        RelationshipType::WorksFor => ("WORKS_FOR", "Person", "Company"),
        RelationshipType::WorksWith => ("WORKS_WITH", "Person", "Person"),
        RelationshipType::WorksOn => ("WORKS_ON", "Person", "Project"),
        RelationshipType::ReportsTo => ("REPORTS_TO", "Person", "Person"),
        RelationshipType::Leads => ("LEADS", "Person", "Project"),
        RelationshipType::ExpertIn => ("EXPERT_IN", "Person", "Topic"),
        RelationshipType::LocatedIn => ("LOCATED_IN", "Person", "Location"),
        RelationshipType::PartnersWith => ("PARTNERS_WITH", "Company", "Company"),
        RelationshipType::RelatedTo => ("RELATED_TO", "Person", "Topic"),
        // Unsupported types — no matching Kuzu table
        _ => ("", "", ""),
    }
}

fn rel_type_from_table(table: &str) -> RelationshipType {
    match table {
        "WORKS_FOR" => RelationshipType::WorksFor,
        "WORKS_WITH" => RelationshipType::WorksWith,
        "WORKS_ON" => RelationshipType::WorksOn,
        "REPORTS_TO" => RelationshipType::ReportsTo,
        "LEADS" => RelationshipType::Leads,
        "EXPERT_IN" => RelationshipType::ExpertIn,
        "LOCATED_IN" => RelationshipType::LocatedIn,
        "PARTNERS_WITH" => RelationshipType::PartnersWith,
        _ => RelationshipType::RelatedTo,
    }
}

fn entity_type_from_label(label: &str) -> EntityType {
    match label {
        "Company" => EntityType::Company,
        "Project" => EntityType::Project,
        "Tool" => EntityType::Tool,
        "Topic" => EntityType::Topic,
        "Location" => EntityType::Location,
        _ => EntityType::Person,
    }
}

// --- value extraction helpers ---

fn value_to_string(v: &kuzu::Value) -> String {
    match v {
        kuzu::Value::String(s) => s.clone(),
        other => format!("{}", other),
    }
}

fn value_to_f64(v: &kuzu::Value) -> f64 {
    match v {
        kuzu::Value::Double(d) => *d,
        kuzu::Value::Float(f) => *f as f64,
        kuzu::Value::Int64(i) => *i as f64,
        kuzu::Value::Int32(i) => *i as f64,
        _ => 0.0,
    }
}

fn parse_dt(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now())
}

/// Escape a string for use in a Kuzu Cypher query (single-quote escape).
fn escape_str(s: &str) -> String {
    s.replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;
    use uuid::Uuid;

    fn open_temp_store() -> (TempDir, GraphStore) {
        let dir = TempDir::new().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        (dir, store)
    }

    fn make_person(name: &str) -> Entity {
        let now = Utc::now();
        Entity {
            id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            entity_type: EntityType::Person,
            value: name.to_string(),
            normalized: name.to_lowercase(),
            confidence: 0.9,
            source: "test".to_string(),
            source_id: None,
            context: None,
            aliases: vec![],
            occurrence_count: 1,
            first_seen: now,
            last_seen: now,
            created_at: now,
        }
    }

    #[test]
    fn test_migrate_is_idempotent() {
        let (_dir, store) = open_temp_store();
        // Second call must not error
        store.migrate().unwrap();
        store.migrate().unwrap();
    }

    #[test]
    fn test_upsert_and_search_entity() {
        let (_dir, store) = open_temp_store();
        let alice = make_person("Alice Testington");
        store.upsert_entity(&alice).unwrap();

        let results = store.search_entities("Alice", 10).unwrap();
        assert!(
            results.iter().any(|e| e.id == alice.id),
            "expected Alice in results"
        );
    }

    #[test]
    fn test_upsert_relationship() {
        let (_dir, store) = open_temp_store();
        let now = Utc::now();

        let person = make_person("Bob");
        let company = Entity {
            id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            entity_type: EntityType::Company,
            value: "Acme Corp".to_string(),
            normalized: "acme corp".to_string(),
            confidence: 0.9,
            source: "test".to_string(),
            source_id: None,
            context: None,
            aliases: vec![],
            occurrence_count: 1,
            first_seen: now,
            last_seen: now,
            created_at: now,
        };

        store.upsert_entity(&person).unwrap();
        store.upsert_entity(&company).unwrap();

        let rel = Relationship {
            id: Uuid::new_v4(),
            user_id: "user1".to_string(),
            from_entity_type: EntityType::Person,
            from_entity_value: person.id.to_string(),
            to_entity_type: EntityType::Company,
            to_entity_value: company.id.to_string(),
            relationship_type: RelationshipType::WorksFor,
            confidence: 0.8,
            evidence: Some("Bob works at Acme Corp".to_string()),
            source_id: None,
            status: RelationshipStatus::Active,
            first_seen: now,
            last_seen: now,
        };

        store.upsert_relationship(&rel).unwrap();

        let rels = store
            .get_entity_relationships(&person.id.to_string())
            .unwrap();
        assert!(
            rels.iter()
                .any(|r| r.to_entity_value == company.id.to_string()),
            "expected relationship to Acme Corp"
        );
    }
}
