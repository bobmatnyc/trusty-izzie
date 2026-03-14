//! Secrets management via macOS Keychain (keyring crate).
//!
//! `get(key)` tries Keychain first, falls back to env var.
//! `set(key, value)` writes to Keychain.
//! `delete(key)` removes from Keychain.
//!
//! The env-var fallback means existing .env files keep working.

const SERVICE: &str = "trusty-izzie";

pub fn get(key: &str) -> Option<String> {
    // Try Keychain first
    if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
        if let Ok(val) = entry.get_password() {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    // Fall back to environment variable
    std::env::var(key).ok()
}

pub fn set(key: &str, value: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new(SERVICE, key)?;
    entry.set_password(value)
}

pub fn delete(key: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new(SERVICE, key)?;
    entry.delete_credential()
}

/// Migrate: read all known secret env vars and write them to Keychain.
/// Safe to call repeatedly — subsequent calls are no-ops for already-stored keys.
pub fn migrate_from_env() {
    for key in SECRET_KEYS {
        if let Ok(val) = std::env::var(key) {
            if !val.is_empty() {
                let _ = set(key, &val);
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
