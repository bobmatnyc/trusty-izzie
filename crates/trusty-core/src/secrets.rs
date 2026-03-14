//! Secrets management — reads from environment variables.
//!
//! Secrets are loaded at startup via dotenvy from .env / config.env.
//! The file is chmod 600 (user-read-only). No Keychain, no prompts.
//!
//! `get(key)` reads from the current process environment.
//! `set(key, value)` writes to runtime env AND appends to config.env.
//! `migrate_from_env()` is a no-op (kept for API compatibility).

use std::path::PathBuf;

fn config_env_path() -> PathBuf {
    let data_dir = std::env::var("TRUSTY_DATA_DIR")
        .unwrap_or_else(|_| "~/.local/share/trusty-izzie".to_string());
    // Manual tilde expansion (no external dep)
    let expanded = if data_dir.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            format!("{}{}", home, &data_dir[1..])
        } else {
            data_dir
        }
    } else {
        data_dir
    };
    PathBuf::from(expanded).join("config.env")
}

/// Read a secret from the process environment.
pub fn get(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Persist a secret to runtime env and config.env.
pub fn set(key: &str, value: &str) -> Result<(), String> {
    std::env::set_var(key, value);

    let path = config_env_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let prefix = format!("{}=", key);
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| !l.starts_with(&prefix))
        .map(String::from)
        .collect();
    lines.push(format!("{}={}", key, value));

    let content = lines.join("\n") + "\n";
    std::fs::write(&path, &content).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Remove a secret from the runtime env (does not modify config.env).
pub fn delete(key: &str) -> Result<(), String> {
    std::env::remove_var(key);
    Ok(())
}

/// No-op — kept for API compatibility.
pub fn migrate_from_env() {}

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
