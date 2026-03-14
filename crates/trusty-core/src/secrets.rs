//! Secrets management via macOS Keychain (security CLI).
//!
//! Items are stored with `-T ""` (allow any application) so rebuilding
//! the binary never triggers a password prompt.
//!
//! `get(key)` tries Keychain first, falls back to env var.
//! `set(key, value)` writes to Keychain with allow-any-app ACL.
//! `delete(key)` removes from Keychain.

const SERVICE: &str = "trusty-izzie";

/// Read a secret: Keychain first, env var fallback.
pub fn get(key: &str) -> Option<String> {
    // Try macOS Keychain via security CLI
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            SERVICE,
            "-a",
            key,
            "-w", // print password only
        ])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }

    // Fall back to environment variable
    std::env::var(key).ok()
}

/// Write a secret to Keychain with allow-any-app ACL (no prompts on read).
pub fn set(key: &str, value: &str) -> Result<(), String> {
    // Delete existing entry first (update = delete + add)
    let _ = std::process::Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE, "-a", key])
        .output();

    // Add with -T "" = trusted by any application
    let status = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            SERVICE,
            "-a",
            key,
            "-w",
            value,
            "-T",
            "", // allow any app, no prompts
        ])
        .status()
        .map_err(|e| e.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "security add-generic-password failed for key {}",
            key
        ))
    }
}

/// Delete a secret from Keychain.
pub fn delete(key: &str) -> Result<(), String> {
    std::process::Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE, "-a", key])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// On first run: migrate any secrets still in env vars into Keychain.
/// Safe to call repeatedly (subsequent calls are no-ops for existing entries).
pub fn migrate_from_env() {
    for key in SECRET_KEYS {
        // Only migrate if not already in Keychain
        let already_stored = std::process::Command::new("security")
            .args(["find-generic-password", "-s", SERVICE, "-a", key])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !already_stored {
            if let Ok(val) = std::env::var(key) {
                if !val.is_empty() {
                    let _ = set(key, &val);
                }
            }
        }
    }
}

pub const SECRET_KEYS: &[&str] = &[
    "OPENROUTER_API_KEY",
    "GOOGLE_CLIENT_SECRET",
    "SLACK_BOT_TOKEN",
    "SLACK_APP_TOKEN",
    "SLACK_USER_TOKEN",
    "SLACK_SIGNING_SECRET",
    "SLACK_CLIENT_SECRET",
    "SLACK_WEBHOOK_URL",
    "SLACK_VERIFICATION_TOKEN",
    "TELEGRAM_BOT_TOKEN",
    "BRAVE_SEARCH_API_KEY",
    "TAVILY_API_KEY",
    "FIRECRAWL_API_KEY",
    "SKYVERN_API_KEY",
    "SERPAPI_API_KEY",
    "WEAVIATE_API_KEY",
];
