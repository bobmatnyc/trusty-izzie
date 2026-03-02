//! Interaction logging — appends structured JSONL records to `{data_dir}/interactions.jsonl`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct InteractionLog<'a> {
    pub ts: String,
    pub command: &'static str,
    pub session_id: Option<String>,
    // chat fields
    pub message: Option<&'a str>,
    pub reply_preview: Option<String>, // first 120 chars of reply
    pub tokens: Option<u32>,
    pub duration_ms: u64,
    pub memories_saved: Option<usize>,
    // search fields
    pub query: Option<&'a str>,
    pub result_count: Option<usize>,
    // test mode
    pub dry_run: bool,
}

/// Appends one JSON line to `path`, creating parent directories if needed.
/// Errors are returned to the caller; the caller should silently ignore them.
pub fn append_interaction(path: &Path, entry: &InteractionLog<'_>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}
