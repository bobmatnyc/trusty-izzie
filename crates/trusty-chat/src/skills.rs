//! Skill loading — reads SKILL.md files from docs/skills/ and injects into system prompt.

use std::path::Path;
use tracing::{info, warn};

/// Load all skill files from the skills directory and return combined markdown.
/// Returns empty string if the directory doesn't exist.
pub fn load_skills(skills_dir: &str) -> String {
    let dir = Path::new(skills_dir);
    if !dir.exists() {
        return String::new();
    }

    let mut skills = Vec::new();

    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md")
                    && path.file_name().and_then(|n| n.to_str()) != Some("README.md")
                {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            info!("Loaded skill: {}", path.display());
                            skills.push(content);
                        }
                        Err(e) => warn!("Failed to read skill {}: {e}", path.display()),
                    }
                }
            }
        }
        Err(e) => warn!("Failed to read skills dir {}: {e}", dir.display()),
    }

    if skills.is_empty() {
        return String::new();
    }

    format!(
        "\n\n## Skills\n\nYou have the following additional skills available:\n\n{}",
        skills.join("\n\n---\n\n")
    )
}
