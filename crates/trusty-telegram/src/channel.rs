//! Abstract channel trait for searching external sources for entity context.

use anyhow::Result;
use async_trait::async_trait;
use trusty_models::entity::Entity;

/// A result from searching a channel for a specific entity.
#[allow(dead_code)]
pub struct ChannelResult {
    /// Title of the found item (document name, email subject, etc.).
    pub title: String,
    /// Context snippet around the entity mention (max 500 chars).
    pub snippet: String,
    /// Optional URL linking to the source item.
    pub url: Option<String>,
}

/// Abstraction over external sources (Google Drive, Gmail, etc.) that can be
/// searched for additional context about a known entity.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Human-readable name identifying this channel (e.g. "gdrive").
    #[allow(dead_code)]
    fn name(&self) -> &'static str;

    /// Search the channel for mentions of `entity`.
    ///
    /// Returns up to `max_results` results ranked by relevance.
    async fn search_for_entity(
        &self,
        entity: &Entity,
        max_results: usize,
    ) -> Result<Vec<ChannelResult>>;
}
