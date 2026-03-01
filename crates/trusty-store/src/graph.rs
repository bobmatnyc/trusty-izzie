//! Kuzu graph database store for entities and relationships.

use anyhow::Result;
use std::path::Path;

use trusty_models::entity::{Entity, Relationship};

/// Handle to the Kuzu embedded graph database.
pub struct GraphStore {
    _db: kuzu::Database,
}

impl GraphStore {
    /// Open (or create) a Kuzu database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;
        let db = kuzu::Database::new(path, kuzu::SystemConfig::default())?;
        Ok(Self { _db: db })
    }

    /// Ensure the schema (node and edge tables) exists.
    pub fn migrate(&self) -> Result<()> {
        todo!("run Kuzu DDL to create Entity and Relationship node/edge tables")
    }

    /// Upsert an entity node into the graph.
    pub fn upsert_entity(&self, _entity: &Entity) -> Result<()> {
        todo!("implement entity upsert via Kuzu Cypher")
    }

    /// Upsert a relationship edge into the graph.
    pub fn upsert_relationship(&self, _relationship: &Relationship) -> Result<()> {
        todo!("implement relationship upsert via Kuzu Cypher")
    }

    /// Find entities by normalised value prefix.
    pub fn search_entities(&self, _query: &str, _limit: usize) -> Result<Vec<Entity>> {
        todo!("implement entity prefix search via Kuzu Cypher")
    }

    /// Fetch all relationships for a given entity (by normalised value).
    pub fn get_entity_relationships(&self, _entity_value: &str) -> Result<Vec<Relationship>> {
        todo!("implement relationship traversal via Kuzu Cypher")
    }
}
