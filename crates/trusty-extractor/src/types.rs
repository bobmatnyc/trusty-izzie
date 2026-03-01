//! Input/output types for the extraction pipeline.

use serde::{Deserialize, Serialize};
use trusty_models::entity::{Entity, Relationship};

/// Context about the authenticated user, used to avoid extracting the
/// user themselves as an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserContext {
    /// Google `sub` claim — stable user identifier.
    pub user_id: String,
    /// The user's email address (to exclude from entity lists).
    pub email: String,
    /// The user's display name (to exclude from entity lists).
    pub display_name: String,
}

/// The structured output produced by one extraction call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    /// Entities identified in this email. May include entities below the
    /// occurrence threshold — the store layer decides what to persist.
    pub entities: Vec<Entity>,
    /// Relationships identified between the entities above.
    pub relationships: Vec<Relationship>,
    /// The model's overall confidence in this extraction.
    pub overall_confidence: f32,
    /// Tokens consumed by the extraction call (for cost tracking).
    pub tokens_used: u32,
    /// Whether the extractor determined this is a marketing/newsletter email
    /// and skipped detailed extraction.
    pub skipped_noise: bool,
}
