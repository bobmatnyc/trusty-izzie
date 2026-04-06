//! Fire-and-forget GitHub issue filing via the `gh` CLI.
//!
//! Any part of the codebase (daemon, chat engine, etc.) can call [`file_issue`]
//! to proactively report errors or improvement opportunities. The function
//! deduplicates by title, rate-limits to at most one issue per 5 minutes, and
//! never propagates errors — failures are logged as warnings.

use std::sync::atomic::{AtomicI64, Ordering};

/// Repo to file issues against.
const REPO: &str = "bobmatnyc/trusty-izzie";

/// Minimum interval between issue creations (seconds).
const RATE_LIMIT_SECS: i64 = 300; // 5 minutes

/// Epoch-second timestamp of the last successfully created issue.
static LAST_FILED: AtomicI64 = AtomicI64::new(0);

/// Files a GitHub issue in the trusty-izzie repo via the `gh` CLI.
///
/// This is fire-and-forget — errors are logged but never propagate.
/// Deduplicates by checking for existing open issues with the same title.
///
/// # Arguments
/// * `title` — Issue title (also used for dedup search).
/// * `body` — Markdown body.
/// * `labels` — Extra labels; `"automated"` is always appended.
pub async fn file_issue(title: &str, body: &str, labels: &[&str]) {
    // --- Rate limiting ---
    let now = chrono::Utc::now().timestamp();
    let prev = LAST_FILED.load(Ordering::Relaxed);
    if now - prev < RATE_LIMIT_SECS {
        tracing::debug!(
            elapsed = now - prev,
            limit = RATE_LIMIT_SECS,
            "Skipping GitHub issue (rate-limited)"
        );
        return;
    }

    // --- Dedup: search for existing open issue with same title ---
    match find_existing_issue(title).await {
        Ok(true) => {
            tracing::debug!(title, "Skipping GitHub issue (duplicate exists)");
            return;
        }
        Ok(false) => {} // no duplicate — proceed
        Err(e) => {
            // If the search itself fails we still try to create the issue.
            tracing::warn!(error = %e, "Failed to check for duplicate GitHub issues");
        }
    }

    // --- Build label list ---
    let mut all_labels: Vec<&str> = labels.to_vec();
    if !all_labels.contains(&"automated") {
        all_labels.push("automated");
    }

    // --- Create the issue ---
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue", "create", "--repo", REPO, "--title", title, "--body", body,
    ]);
    for label in &all_labels {
        cmd.args(["--label", label]);
    }

    match cmd.output().await {
        Ok(output) if output.status.success() => {
            LAST_FILED.store(now, Ordering::Relaxed);
            let url = String::from_utf8_lossy(&output.stdout);
            tracing::info!(url = url.trim(), "Proactively filed GitHub issue");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(stderr = stderr.trim(), "gh issue create failed");
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to run gh CLI");
        }
    }
}

/// Returns `true` if an open issue with a matching title already exists.
async fn find_existing_issue(title: &str) -> Result<bool, String> {
    let output = tokio::process::Command::new("gh")
        .args([
            "issue", "list", "--repo", REPO, "--state", "open", "--search", title, "--json",
            "title", "--limit", "5",
        ])
        .output()
        .await
        .map_err(|e| format!("gh issue list failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue list returned error: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse gh output: {e}"))?;

    let dominated = title.to_lowercase();
    Ok(items.iter().any(|item| {
        item["title"]
            .as_str()
            .is_some_and(|t| t.to_lowercase() == dominated)
    }))
}
