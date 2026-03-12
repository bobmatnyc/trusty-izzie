//! `ScriptSkill` — runs an external script (Python, bash, Node, etc.) as a skill.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::warn;

use crate::skill::{Skill, SkillResult, SkillTool, SkillToolCall};

// ---------------------------------------------------------------------------
// Frontmatter schema
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: String,
    #[serde(default)]
    description: String,
    execute: Option<ExecuteConfig>,
    #[serde(default)]
    tools: Vec<ToolDef>,
}

#[derive(Debug, Deserialize, Clone)]
struct ExecuteConfig {
    runtime: String,
    command: String,
    #[serde(default)]
    arg_format: ArgFormat,
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ArgFormat {
    #[default]
    JsonStdin,
    EnvVars,
}

#[derive(Debug, Deserialize)]
struct ToolDef {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    parameters: serde_json::Value,
}

// ---------------------------------------------------------------------------
// ScriptSkill
// ---------------------------------------------------------------------------

/// A skill backed by an external script process.
pub struct ScriptSkill {
    /// Skill name from frontmatter.
    name: String,
    /// Skill description from frontmatter.
    description: String,
    /// Tools declared in frontmatter.
    tools: Vec<SkillTool>,
    /// Directory containing the skill file — used to resolve relative `command` paths.
    base_dir: PathBuf,
    /// Execution configuration.
    execute: ExecuteConfig,
    /// Optional system-prompt contribution (body of the .md file after frontmatter).
    prompt_contribution: Option<String>,
}

impl ScriptSkill {
    /// Parse a `.md` file with YAML frontmatter into a `ScriptSkill`.
    ///
    /// Returns `None` if the file has no `execute:` key (documentation-only).
    pub fn from_path(path: &Path) -> Option<Self> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                warn!("ScriptSkill: failed to read {:?}: {e}", path);
                return None;
            }
        };

        let (fm, body) = match parse_frontmatter(&raw) {
            Some(pair) => pair,
            None => {
                warn!("ScriptSkill: no YAML frontmatter in {:?}", path);
                return None;
            }
        };

        let execute = fm.execute?;

        let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        let tools = fm
            .tools
            .into_iter()
            .map(|t| SkillTool {
                name: t.name,
                description: t.description,
                parameters: t.parameters,
            })
            .collect();

        let prompt_contribution = if body.trim().is_empty() {
            None
        } else {
            Some(body.to_string())
        };

        Some(Self {
            name: fm.name,
            description: fm.description,
            tools,
            base_dir,
            execute,
            prompt_contribution,
        })
    }
}

#[async_trait]
impl Skill for ScriptSkill {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn tools(&self) -> Vec<SkillTool> {
        self.tools.clone()
    }

    fn system_prompt_contribution(&self) -> Option<String> {
        self.prompt_contribution.clone()
    }

    async fn execute(&self, call: &SkillToolCall) -> Option<SkillResult> {
        // Only handle tools we advertise.
        if !self.tools.iter().any(|t| t.name == call.name) {
            return None;
        }
        Some(self.run_script(call).await)
    }
}

impl ScriptSkill {
    async fn run_script(&self, call: &SkillToolCall) -> SkillResult {
        let cmd_path = self.base_dir.join(&self.execute.command);

        let child = match self.execute.arg_format {
            ArgFormat::JsonStdin => {
                let mut child = Command::new(&self.execute.runtime)
                    .arg(&cmd_path)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .with_context(|| {
                        format!(
                            "ScriptSkill '{}': failed to spawn {} {:?}",
                            self.name, self.execute.runtime, cmd_path
                        )
                    })?;

                // Write JSON args to stdin.
                if let Some(stdin) = child.stdin.take() {
                    let json_bytes = serde_json::to_vec(&call.arguments)?;
                    let mut stdin = stdin;
                    stdin
                        .write_all(&json_bytes)
                        .await
                        .context("writing args to script stdin")?;
                    // stdin dropped here, closing the pipe
                }

                child
            }
            ArgFormat::EnvVars => {
                let mut cmd = Command::new(&self.execute.runtime);
                cmd.arg(&cmd_path)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                if let Some(obj) = call.arguments.as_object() {
                    for (k, v) in obj {
                        let env_key = format!("SKILL_{}", k.to_uppercase());
                        let env_val = match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        cmd.env(env_key, env_val);
                    }
                }

                cmd.spawn().with_context(|| {
                    format!(
                        "ScriptSkill '{}': failed to spawn {} {:?}",
                        self.name, self.execute.runtime, cmd_path
                    )
                })?
            }
        };

        let output = child
            .wait_with_output()
            .await
            .context("waiting for script process")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "ScriptSkill '{}' exited with {}: {stderr}",
                self.name,
                output.status
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

// ---------------------------------------------------------------------------
// Frontmatter parser
// ---------------------------------------------------------------------------

/// Split `---\n<yaml>\n---\n<body>` into `(Frontmatter, body)`.
fn parse_frontmatter(src: &str) -> Option<(Frontmatter, &str)> {
    let src = src.trim_start();
    let src = src.strip_prefix("---")?;
    // Find the closing `---`
    let close = src.find("\n---")?;
    let yaml_str = &src[..close];
    let body = src[close + 4..].trim_start_matches('\n');

    let fm: Frontmatter = match serde_yaml::from_str(yaml_str) {
        Ok(f) => f,
        Err(e) => {
            warn!("ScriptSkill: malformed YAML frontmatter: {e}");
            return None;
        }
    };

    Some((fm, body))
}

// ---------------------------------------------------------------------------
// Directory scanner
// ---------------------------------------------------------------------------

/// Scan `dir` for `*.md` files with an `execute:` key and return `ScriptSkill` instances.
pub fn load_script_skills(dir: &Path) -> Vec<Arc<dyn Skill + Send + Sync>> {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!("load_script_skills: cannot read directory {:?}: {e}", dir);
            return vec![];
        }
    };

    let mut skills: Vec<Arc<dyn Skill + Send + Sync>> = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(skill) = ScriptSkill::from_path(&path) {
            skills.push(Arc::new(skill));
        }
    }

    skills
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"---
name: test-skill
description: "A test skill"
execute:
  runtime: python3
  command: scripts/test.py
  arg_format: json-stdin
tools:
  - name: do_thing
    description: "Does a thing"
    parameters:
      type: object
      properties:
        input:
          type: string
      required: [input]
---
# Test Skill

Some body text here.
"#;

    #[test]
    fn parse_valid_frontmatter() {
        let (fm, body) = parse_frontmatter(SAMPLE).expect("should parse");
        assert_eq!(fm.name, "test-skill");
        assert_eq!(fm.tools.len(), 1);
        assert_eq!(fm.tools[0].name, "do_thing");
        assert!(body.contains("Test Skill"));
    }

    const DOC_ONLY: &str = r#"---
name: docs-only
description: "No execute key"
tools: []
---
# Docs Only
"#;

    #[test]
    fn doc_only_returns_none() {
        let (fm, _) = parse_frontmatter(DOC_ONLY).expect("should parse yaml");
        assert!(fm.execute.is_none());
    }

    #[test]
    fn load_empty_dir_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = load_script_skills(tmp.path());
        assert!(skills.is_empty());
    }
}
