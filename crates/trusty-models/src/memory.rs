//! Memory types for the personal knowledge store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Semantic category of a stored memory, used for retrieval filtering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// A preference, habit, or personal style the user exhibits.
    UserPreference,
    /// A fact about a person the user knows.
    PersonFact,
    /// A fact about a project or initiative.
    ProjectFact,
    /// A fact about a company or organisation.
    CompanyFact,
    /// A recurring event, meeting, or commitment.
    RecurringEvent,
    /// A decision made by the user or their team.
    Decision,
    /// A notable event that occurred.
    Event,
    /// A time-based reminder or follow-up the user wants tracked.
    Reminder,
    /// A physical location, address, or place the user cares about.
    Location,
    /// A contact method or account handle for a person.
    Contact,
    /// Any other memory that does not fit above categories.
    /// Also used as a fallback for any unrecognised category string.
    #[serde(other)]
    General,
}

/// Approximate decay rate for memory relevance scoring.
/// Higher values mean the memory stays relevant longer.
impl MemoryCategory {
    /// Half-life in days for relevance decay calculations.
    pub fn decay_half_life_days(&self) -> f32 {
        match self {
            MemoryCategory::UserPreference => 365.0,
            MemoryCategory::PersonFact => 180.0,
            MemoryCategory::ProjectFact => 90.0,
            MemoryCategory::CompanyFact => 180.0,
            MemoryCategory::RecurringEvent => 30.0,
            MemoryCategory::Decision => 120.0,
            MemoryCategory::Event => 60.0,
            MemoryCategory::Reminder => 14.0,
            MemoryCategory::Location => 365.0,
            MemoryCategory::Contact => 365.0,
            MemoryCategory::General => 90.0,
        }
    }
}

/// A single stored memory item in the knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Stable primary key.
    pub id: Uuid,
    /// Owner of this memory.
    pub user_id: String,
    /// Semantic category for filtering and decay.
    pub category: MemoryCategory,
    /// Human-readable summary of the memory.
    pub content: String,
    /// Dense embedding vector for semantic similarity search.
    pub embedding: Option<Vec<f32>>,
    /// Related entity values (normalised) for graph traversal.
    pub related_entities: Vec<String>,
    /// Source email or event ID that generated this memory.
    pub source_id: Option<String>,
    /// Importance weight in `[0.0, 1.0]`.
    pub importance: f32,
    /// Number of times this memory has been retrieved and used.
    pub access_count: u32,
    /// Timestamp of last access (for recency scoring).
    pub last_accessed: Option<DateTime<Utc>>,
    /// Record creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}
