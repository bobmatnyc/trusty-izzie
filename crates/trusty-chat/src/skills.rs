//! Skill loading — parses SKILL.md files from docs/skills/ and injects structured
//! tool awareness into Izzie's system prompt.

use serde::Deserialize;
use std::path::Path;
use tracing::{info, warn};

/// Frontmatter parsed from a SKILL.md file.
#[derive(Debug, Deserialize, Default)]
struct SkillFrontmatter {
    name: Option<String>,
    #[allow(dead_code)]
    version: Option<String>,
    description: Option<String>,
    #[serde(default)]
    tools: Vec<SkillTool>,
    #[serde(default)]
    examples: Vec<String>,
}

/// A tool definition within a skill's frontmatter.
#[derive(Debug, Deserialize)]
struct SkillTool {
    name: String,
    description: Option<String>,
    #[serde(default)]
    parameters: std::collections::HashMap<String, ToolParam>,
}

/// A single parameter definition.
#[derive(Debug, Deserialize)]
struct ToolParam {
    #[serde(rename = "type")]
    param_type: Option<String>,
    description: Option<String>,
    example: Option<serde_yaml::Value>,
    default: Option<serde_yaml::Value>,
    #[serde(rename = "enum", default)]
    enum_values: Vec<String>,
}

/// Render a serde_yaml::Value as a compact human-readable string.
fn yaml_value_to_string(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Null => "null".to_string(),
        other => format!("{other:?}"),
    }
}

/// Parse frontmatter and body from a skill file.
/// Returns (frontmatter_yaml, body_markdown).
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return (None, content);
    }
    let after_open = &content[3..];
    if let Some(close_pos) = after_open.find("\n---") {
        let fm = after_open[..close_pos].trim();
        let body = after_open[close_pos + 4..].trim_start();
        (Some(fm), body)
    } else {
        (None, content)
    }
}

/// Format a single skill's frontmatter into system prompt text.
fn format_skill(fm: &SkillFrontmatter, body: &str) -> String {
    let name = fm.name.as_deref().unwrap_or("unnamed");
    let desc = fm.description.as_deref().unwrap_or("");

    let mut out = format!("### Skill: {name}\n{desc}\n");

    if !fm.tools.is_empty() {
        out.push_str("\nAvailable tools:\n");
        for tool in &fm.tools {
            out.push_str(&format!("\n**{}**", tool.name));
            if let Some(d) = &tool.description {
                out.push_str(&format!(": {d}"));
            }
            out.push('\n');
            if !tool.parameters.is_empty() {
                out.push_str("Parameters:\n");
                for (pname, param) in &tool.parameters {
                    let ptype = param.param_type.as_deref().unwrap_or("string");
                    let pdesc = param.description.as_deref().unwrap_or("");
                    out.push_str(&format!("- `{pname}` ({ptype}): {pdesc}"));
                    if let Some(ex) = &param.example {
                        out.push_str(&format!(" — e.g. {}", yaml_value_to_string(ex)));
                    }
                    if let Some(def) = &param.default {
                        out.push_str(&format!(" (default: {})", yaml_value_to_string(def)));
                    }
                    if !param.enum_values.is_empty() {
                        out.push_str(&format!(" [{}]", param.enum_values.join(", ")));
                    }
                    out.push('\n');
                }
            }
        }
    }

    if !fm.examples.is_empty() {
        out.push_str("\nExample queries:\n");
        for ex in &fm.examples {
            out.push_str(&format!("- \"{ex}\"\n"));
        }
    }

    let body = body.trim();
    if !body.is_empty() && body.len() < 2000 {
        out.push_str(&format!("\n{body}\n"));
    }

    out
}

/// Load all skill files from the skills directory and return combined markdown
/// for injection into the system prompt.
/// Returns empty string if the directory doesn't exist or no skills are found.
pub fn load_skills(skills_dir: &str) -> String {
    let dir = Path::new(skills_dir);
    if !dir.exists() {
        return String::new();
    }

    let mut skill_sections = Vec::new();

    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(e) => e.flatten().collect(),
        Err(e) => {
            warn!("Failed to read skills dir {}: {e}", dir.display());
            return String::new();
        }
    };
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("README.md") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let (fm_yaml, body) = split_frontmatter(&content);
                let formatted = if let Some(yaml) = fm_yaml {
                    match serde_yaml::from_str::<SkillFrontmatter>(yaml) {
                        Ok(fm) => {
                            info!(
                                "Loaded skill '{}' from {}",
                                fm.name.as_deref().unwrap_or("?"),
                                path.display()
                            );
                            format_skill(&fm, body)
                        }
                        Err(e) => {
                            warn!(
                                "Failed to parse skill frontmatter in {}: {e}",
                                path.display()
                            );
                            content.clone()
                        }
                    }
                } else {
                    info!("Loaded skill (no frontmatter) from {}", path.display());
                    content.clone()
                };
                skill_sections.push(formatted);
            }
            Err(e) => warn!("Failed to read skill {}: {e}", path.display()),
        }
    }

    if skill_sections.is_empty() {
        return String::new();
    }

    format!(
        "\n\n## Active Skills\n\nYou have the following skills available. Use the listed tools when the user's request matches.\n\n{}",
        skill_sections.join("\n---\n\n")
    )
}
