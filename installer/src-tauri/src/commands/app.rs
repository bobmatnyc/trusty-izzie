use std::collections::HashMap;

#[derive(serde::Serialize, Default)]
pub struct AppConfig {
    pub llm_provider: String,
    pub has_openrouter_key: bool,
    pub aws_region: Option<String>,
    pub slack_mode: String,
    pub google_email: Option<String>,
    pub data_dir: String,
    pub skills_enabled: Vec<String>,
}

/// Check if Izzie is already installed (config.env exists)
#[tauri::command]
pub async fn check_installed() -> Result<bool, String> {
    let path = dirs::home_dir()
        .ok_or("no home dir")?
        .join(".config/trusty-izzie/config.env");
    Ok(path.exists())
}

/// Read existing config.env into AppConfig
#[tauri::command]
pub async fn read_config() -> Result<AppConfig, String> {
    let path = dirs::home_dir()
        .ok_or("no home dir")?
        .join(".config/trusty-izzie/config.env");

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| e.to_string())?;

    let env: HashMap<String, String> = content
        .lines()
        .filter(|l| !l.starts_with('#') && l.contains('='))
        .filter_map(|l| {
            let mut parts = l.splitn(2, '=');
            Some((
                parts.next()?.trim().to_string(),
                parts.next()?.trim().to_string(),
            ))
        })
        .collect();

    let data_dir = env
        .get("TRUSTY_DATA_DIR")
        .cloned()
        .unwrap_or_else(|| "~/.local/share/trusty-izzie".to_string());

    Ok(AppConfig {
        llm_provider: if env.contains_key("OPENROUTER_API_KEY") {
            "openrouter".to_string()
        } else if env.contains_key("AWS_REGION") {
            "bedrock".to_string()
        } else {
            "unknown".to_string()
        },
        has_openrouter_key: env.contains_key("OPENROUTER_API_KEY"),
        aws_region: env.get("AWS_REGION").cloned(),
        slack_mode: if env.contains_key("SLACK_BOT_TOKEN") {
            "self".to_string()
        } else if env.contains_key("TRUSTY_ROUTER_URL") {
            "managed".to_string()
        } else {
            "skip".to_string()
        },
        google_email: env.get("TRUSTY_PRIMARY_EMAIL").cloned(),
        data_dir,
        skills_enabled: env
            .get("TRUSTY_SKILLS_ENABLED")
            .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
            .unwrap_or_default(),
    })
}

/// Open a path in Finder
#[tauri::command]
pub async fn open_in_finder(path: String) -> Result<(), String> {
    let expanded = shellexpand::tilde(&path).to_string();
    tokio::process::Command::new("open")
        .arg(&expanded)
        .status()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Reset config (delete config.env only, never data dir)
#[tauri::command]
pub async fn reset_config() -> Result<(), String> {
    let path = dirs::home_dir()
        .ok_or("no home dir")?
        .join(".config/trusty-izzie/config.env");
    if path.exists() {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Update skills in existing config.env
#[tauri::command]
pub async fn update_skills(
    enabled: Vec<String>,
    keys: std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let path = dirs::home_dir()
        .ok_or("no home dir")?
        .join(".config/trusty-izzie/config.env");

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| e.to_string())?;

    let skill_key_envs: std::collections::HashSet<&str> = [
        "TRUSTY_SKILLS_ENABLED",
        "TAVILY_API_KEY",
        "FIRECRAWL_API_KEY",
        "BRAVE_API_KEY",
        "GITHUB_TOKEN",
        "LINEAR_API_KEY",
        "NOTION_API_KEY",
        "SPOTIFY_CLIENT_ID",
        "SPOTIFY_CLIENT_SECRET",
    ]
    .iter()
    .cloned()
    .collect();

    let mut new_lines: Vec<String> = content
        .lines()
        .filter(|l| {
            if l.starts_with('#') {
                return true;
            }
            let key = l.split('=').next().unwrap_or("").trim();
            !skill_key_envs.contains(key)
        })
        .map(|l| l.to_string())
        .collect();

    new_lines.push(String::new());
    new_lines.push("# Skills".to_string());
    new_lines.push(format!("TRUSTY_SKILLS_ENABLED={}", enabled.join(",")));
    let mut sorted: Vec<_> = keys.iter().collect();
    sorted.sort_by_key(|(k, _)| k.as_str());
    for (env, val) in sorted {
        new_lines.push(format!("{env}={val}"));
    }

    tokio::fs::write(&path, new_lines.join("\n") + "\n")
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
