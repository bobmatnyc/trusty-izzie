//! Entity and relationship types extracted from email and calendar.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The category of a named entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// A human individual.
    Person,
    /// An organisation, business, or institution.
    Company,
    /// A work project or initiative.
    Project,
    /// A tool, product, or technology.
    Tool,
    /// A subject-matter topic or domain.
    Topic,
    /// A physical or virtual location.
    Location,
    /// A task or action item.
    ActionItem,
}

/// A named entity extracted from one or more email/calendar sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Stable primary key.
    pub id: Uuid,
    /// Owner of this entity record — Google sub claim.
    pub user_id: String,
    /// Semantic category of the entity.
    pub entity_type: EntityType,
    /// Raw surface form as it appeared in the source.
    pub value: String,
    /// Canonical snake_case form used for de-duplication.
    pub normalized: String,
    /// LLM confidence score in the range `[0.0, 1.0]`.
    pub confidence: f32,
    /// Where the entity was found: `"header"`, `"body"`, or `"calendar"`.
    pub source: String,
    /// The email message ID or calendar event ID that yielded this entity.
    pub source_id: Option<String>,
    /// A short surrounding text snippet for provenance.
    pub context: Option<String>,
    /// Alternative surface forms (abbreviations, nicknames, etc.).
    pub aliases: Vec<String>,
    /// Number of distinct sources in which this entity appeared.
    /// Entities are only persisted once this reaches the configured minimum.
    pub occurrence_count: u32,
    /// Timestamp of the earliest observation.
    pub first_seen: DateTime<Utc>,
    /// Timestamp of the most recent observation.
    pub last_seen: DateTime<Utc>,
    /// Record creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// A directed semantic relationship between two entities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationshipType {
    /// Person A collaborates with Person B.
    WorksWith,
    /// Person works for / is employed by Company.
    WorksFor,
    /// Person or Team is working on a Project.
    WorksOn,
    /// Person reports to another Person.
    ReportsTo,
    /// Person leads a Team or Project.
    Leads,
    /// Person has expertise in a Topic or Tool.
    ExpertIn,
    /// Entity is located in a Location.
    LocatedIn,
    /// Entity is a partner of another Entity.
    PartnersWith,
    /// Generic semantic relationship.
    RelatedTo,
    /// Entity depends on another Entity (projects, tools).
    DependsOn,
    /// Entity is a part of a larger Entity.
    PartOf,
    /// Person has a personal friendship with another Person.
    FriendOf,
    /// Person or Company owns a Tool, Project, or resource.
    Owns,
}

/// Lifecycle state of a relationship.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipStatus {
    /// Relationship is current and believed to be active.
    Active,
    /// Relationship existed in the past but appears to have ended.
    Former,
    /// Insufficient evidence to determine current state.
    Unknown,
}

/// A directed edge in the knowledge graph between two entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Stable primary key.
    pub id: Uuid,
    /// Owner of this relationship record.
    pub user_id: String,
    /// Type of the source entity.
    pub from_entity_type: EntityType,
    /// Normalized value of the source entity.
    pub from_entity_value: String,
    /// Type of the target entity.
    pub to_entity_type: EntityType,
    /// Normalized value of the target entity.
    pub to_entity_value: String,
    /// Semantic label for this edge.
    pub relationship_type: RelationshipType,
    /// LLM confidence in this relationship, `[0.0, 1.0]`.
    pub confidence: f32,
    /// Verbatim text snippet that supports this relationship claim.
    pub evidence: Option<String>,
    /// Source email or event that yielded this relationship.
    pub source_id: Option<String>,
    /// Current lifecycle status.
    pub status: RelationshipStatus,
    /// Timestamp of the earliest observation.
    pub first_seen: DateTime<Utc>,
    /// Timestamp of the most recent observation.
    pub last_seen: DateTime<Utc>,
}
