//! Google Drive channel — search and export documents for entity enrichment.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;
use trusty_models::entity::Entity;
use trusty_store::Store;

use crate::channel::{Channel, ChannelResult};

/// Implements the `Channel` trait against the Google Drive API.
pub struct GDriveChannel {
    http: reqwest::Client,
    access_token: String,
}

// ---------------------------------------------------------------------------
// Drive API response shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DriveFileList {
    files: Option<Vec<DriveFile>>,
}

#[derive(Deserialize)]
struct DriveFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    #[serde(rename = "webViewLink")]
    web_view_link: Option<String>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl GDriveChannel {
    pub fn new(access_token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            access_token,
        }
    }

    /// Export a Google Doc as plain text.
    ///
    /// Calls `GET /drive/v3/files/{id}/export?mimeType=text/plain`.
    pub async fn export_doc_text(&self, file_id: &str) -> Result<String> {
        let url = format!(
            "https://www.googleapis.com/drive/v3/files/{}/export",
            file_id
        );
        let text = self
            .http
            .get(&url)
            .bearer_auth(&self.access_token)
            .query(&[("mimeType", "text/plain")])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(text)
    }

    /// Find a ±200 char window around the first occurrence of `needle` in `haystack`.
    fn context_window(haystack: &str, needle: &str) -> String {
        let hl = haystack.to_lowercase();
        let nl = needle.to_lowercase();
        if let Some(pos) = hl.find(&nl) {
            let start = pos.saturating_sub(200);
            let end = (pos + needle.len() + 200).min(haystack.len());
            let snippet = &haystack[start..end];
            // Truncate to 500 chars for the ChannelResult contract.
            snippet.chars().take(500).collect()
        } else {
            haystack.chars().take(500).collect()
        }
    }
}

#[async_trait]
impl Channel for GDriveChannel {
    fn name(&self) -> &'static str {
        "gdrive"
    }

    async fn search_for_entity(
        &self,
        entity: &Entity,
        max_results: usize,
    ) -> Result<Vec<ChannelResult>> {
        // Escape single quotes for the Drive query syntax.
        let query_value = entity.value.replace('\'', "\\'");
        let url = "https://www.googleapis.com/drive/v3/files";

        let list: DriveFileList = self
            .http
            .get(url)
            .bearer_auth(&self.access_token)
            .query(&[
                ("q", format!("fullText contains '{}'", query_value).as_str()),
                ("fields", "files(id,name,mimeType,webViewLink)"),
                ("pageSize", &max_results.to_string()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let files = list.files.unwrap_or_default();
        let mut results = Vec::with_capacity(files.len());

        for file in files {
            let snippet = if file.mime_type == "application/vnd.google-apps.document" {
                // Export content and find context window.
                match self.export_doc_text(&file.id).await {
                    Ok(text) => Self::context_window(&text, &entity.value),
                    Err(e) => {
                        warn!(file_id = %file.id, error = %e, "failed to export Drive doc");
                        String::new()
                    }
                }
            } else {
                // For non-Docs files, use the file name as context.
                String::new()
            };

            results.push(ChannelResult {
                title: file.name,
                snippet,
                url: file.web_view_link,
            });
        }

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Background enrichment helper
// ---------------------------------------------------------------------------

/// Spawn a fire-and-forget task that searches Google Drive for the entity
/// and enriches its `context` field in LanceDB.
///
/// Failures are silent (logged at WARN level).
pub fn spawn_drive_enrichment(entity: Entity, token: String, store: Arc<Store>) {
    tokio::spawn(async move {
        let ch = GDriveChannel::new(token);
        let results = match ch.search_for_entity(&entity, 3).await {
            Ok(r) => r,
            Err(e) => {
                warn!(entity = %entity.normalized, error = %e, "Drive enrichment search failed");
                return;
            }
        };

        if results.is_empty() {
            return;
        }

        let note = results
            .iter()
            .map(|r| format!("[{}] {}", r.title, r.snippet))
            .collect::<Vec<_>>()
            .join(" | ");

        let mut enriched = entity;
        enriched.context = Some(match &enriched.context {
            Some(c) => format!("{c} | Drive: {note}"),
            None => format!("Drive: {note}"),
        });

        if let Err(e) = store
            .lance
            .upsert_entity(&enriched, vec![0.0f32; 384])
            .await
        {
            warn!(entity = %enriched.normalized, error = %e, "Drive enrichment write to LanceDB failed");
        }
    });
}
