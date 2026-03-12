# Skill Architecture

## Overview

A **skill** is a self-contained Rust crate that:
1. Declares a set of LLM-callable tools (name, description, JSON schema)
2. Executes those tools when invoked
3. Optionally contributes a block of text to the system prompt
4. Optionally exposes itself as a standalone MCP server

The `trusty-chat` engine discovers skills at startup via dependency injection — no changes to `trusty-chat` required when adding a new skill. The hard-coded `ToolName` enum is replaced by a split between **core tools** (built-in to `trusty-chat`) and **skill tools** (dynamically registered by skill crates).

---

## New Crate: `trusty-skill`

Location: `crates/trusty-skill/`

### Public API

```rust
use async_trait::async_trait;
use serde_json::Value;

/// One tool a skill exposes to the LLM.
#[derive(Debug, Clone)]
pub struct SkillTool {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters: Value,
}

/// An incoming tool call from the LLM destined for a skill.
#[derive(Debug, Clone)]
pub struct SkillToolCall {
    pub name: String,
    pub arguments: Value,
}

/// The result of executing a skill tool.
pub type SkillResult = anyhow::Result<String>;

/// Core trait every skill crate must implement.
#[async_trait]
pub trait Skill: Send + Sync + 'static {
    /// Unique kebab-case identifier, e.g. "metro-north", "weather".
    fn name(&self) -> &str;

    /// One-line human description, shown in the skills directory.
    fn description(&self) -> &str;

    /// Tools this skill exposes to the LLM.
    fn tools(&self) -> Vec<SkillTool>;

    /// Optional markdown injected into the system prompt under "## Skills".
    /// Return None if the skill needs no prompt context.
    fn system_prompt_contribution(&self) -> Option<String> {
        None
    }

    /// Execute a tool call. Return `None` if this skill doesn't handle `call.name`.
    async fn execute(&self, call: &SkillToolCall) -> Option<SkillResult>;

    /// Whether this skill can also run as a standalone MCP server.
    fn mcp_capable(&self) -> bool {
        false
    }
}
```

### `ScriptSkill` — Lightweight Adapter

For skills that don't need to be Rust crates, `trusty-skill` provides `ScriptSkill`, which reads a SKILL.md with extended frontmatter and executes tools by spawning a subprocess:

```yaml
# docs/skills/my-tool.md
---
name: my-tool
version: "1.0"
description: "Does something custom"
execute:
  runtime: python3          # or: bash, node, ruby, binary
  command: scripts/my_tool.py
  arg_format: json-stdin    # or: env-vars, positional
tools:
  - name: do_something
    description: "Does something"
    parameters:
      type: object
      properties:
        input: { type: string }
      required: [input]
---
# My Tool

Details about what this skill does and when to use it.
```

`ScriptSkill` implements `Skill`:
- `tools()` reads the YAML frontmatter
- `execute()` spawns `runtime command`, writes JSON args to stdin, reads stdout as result
- Tools can also pass args as environment variables (`arg_format: env-vars`)

This lets Izzie skills be written in Python, shell, or any scripting language without touching Rust.

---

## Refactored `trusty-chat`

### Split `ToolName` into Core + Dynamic

```rust
// crates/trusty-chat/src/tools.rs

/// Built-in tools that trusty-chat handles directly (no skill crate required).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreTool {
    SearchMemories,
    SearchEntities,
    GetEntityRelationships,
    SaveMemory,
    ScheduleEvent,
    CancelEvent,
    ListEvents,
    CheckServiceStatus,
    GetVersion,
    SubmitGithubIssue,
    ListAgents,
    RunAgent,
    GetAgentTask,
    ListAccounts,
    AddAccount,
    RemoveAccount,
    SyncContacts,
    SyncMessages,
    SyncWhatsApp,
    ExecuteShellCommand,
    GetCalendarEvents,
    GetPreferences,
    SetPreference,
    AddVipContact,
    RemoveVipContact,
    ListVipContacts,
    AddWatchSubscription,
    RemoveWatchSubscription,
    ListWatchSubscriptions,
    ListOpenLoops,
    DismissOpenLoop,
    GetTaskLists,
    GetTasks,
    GetTasksBulk,
    SearchImessages,
    SearchContacts,
    SearchWhatsapp,
    CreateCalendarEvent,
    CompleteTask,
    SearchSkills,
    WebSearch,
    FetchPage,
}

/// A tool call from the LLM — either a built-in core tool or a skill tool.
#[derive(Debug, Clone)]
pub enum ToolCall {
    Core { name: CoreTool, arguments: serde_json::Value },
    Skill { name: String, arguments: serde_json::Value },
}
```

### `ChatEngine` with Skill Registry

```rust
pub struct ChatEngine {
    // existing fields ...
    skills: Vec<Arc<dyn Skill>>,
}

impl ChatEngine {
    pub fn with_skills(mut self, skills: Vec<Arc<dyn Skill>>) -> Self {
        self.skills = skills;
        self
    }

    /// All tools visible to the LLM = core tools + skill tools.
    pub fn all_tools(&self) -> Vec<serde_json::Value> {
        let mut tools: Vec<_> = core_tool_definitions(); // existing
        for skill in &self.skills {
            for t in skill.tools() {
                tools.push(serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                }));
            }
        }
        tools
    }

    /// System prompt skill contributions.
    fn skill_context(&self) -> String {
        let mut sections = vec![];
        for skill in &self.skills {
            if let Some(block) = skill.system_prompt_contribution() {
                sections.push(block);
            }
        }
        if sections.is_empty() {
            String::new()
        } else {
            format!("\n\n## Active Skills\n\n{}", sections.join("\n\n"))
        }
    }

    /// Dispatch a tool call — core first, then skills.
    async fn dispatch_tool(&self, call: ToolCall) -> String {
        match call {
            ToolCall::Core { name, arguments } => {
                self.execute_core_tool(&name, &arguments).await
            }
            ToolCall::Skill { name, arguments } => {
                let skill_call = SkillToolCall { name: name.clone(), arguments };
                for skill in &self.skills {
                    if let Some(result) = skill.execute(&skill_call).await {
                        return result.unwrap_or_else(|e| format!("Error: {e}"));
                    }
                }
                format!("Unknown tool: {name}")
            }
        }
    }
}
```

### Tool Call Parsing

The LLM returns a tool name as a string. Parsing tries `CoreTool` first, falls back to `ToolCall::Skill`:

```rust
fn parse_tool_call(name: &str, arguments: Value) -> ToolCall {
    if let Ok(core) = serde_json::from_value::<CoreTool>(
        serde_json::Value::String(name.to_string())
    ) {
        ToolCall::Core { name: core, arguments }
    } else {
        ToolCall::Skill { name: name.to_string(), arguments }
    }
}
```

---

## Skill Crate: `trusty-metro-north`

Add a `MetroNorthSkill` struct in `crates/trusty-metro-north/src/skill.rs`:

```rust
pub struct MetroNorthSkill {
    client: Arc<MetroNorthClient>,
}

#[async_trait]
impl Skill for MetroNorthSkill {
    fn name(&self) -> &str { "metro-north" }
    fn description(&self) -> &str {
        "Real-time Metro North Railroad train schedules and service alerts"
    }
    fn tools(&self) -> Vec<SkillTool> {
        vec![
            SkillTool {
                name: "get_train_schedule".into(),
                description: "Fetch upcoming Metro North train departures".into(),
                parameters: json!({ ... }),
            },
            SkillTool {
                name: "get_train_alerts".into(),
                description: "Fetch current Metro North service alerts".into(),
                parameters: json!({ ... }),
            },
        ]
    }
    fn system_prompt_contribution(&self) -> Option<String> {
        Some(include_str!("../../../docs/skills/metro-north.md").to_string())
        // Or embed the content directly in the source
    }
    async fn execute(&self, call: &SkillToolCall) -> Option<SkillResult> {
        match call.name.as_str() {
            "get_train_schedule" => Some(self.client.get_schedule(&call.arguments).await),
            "get_train_alerts"   => Some(self.client.get_alerts(&call.arguments).await),
            _ => None,
        }
    }
    fn mcp_capable(&self) -> bool { true }
}
```

Same pattern for `trusty-weather` → `WeatherSkill`.

---

## Wiring Skills at Startup

### `trusty-daemon`

```rust
// crates/trusty-daemon/src/main.rs
let skills: Vec<Arc<dyn Skill>> = vec![
    Arc::new(MetroNorthSkill::new()),
    Arc::new(WeatherSkill::new()),
    // Add new skills here — no changes to trusty-chat required
];
let engine = ChatEngine::new(config, store, ...).with_skills(skills);
```

### `trusty-api`

Same pattern — skills injected at `ChatEngine::new()`.

### `trusty-mcp`

MCP tools exposed = all core tools + all skill tools via `engine.all_tools()`.
Dispatch goes through `engine.dispatch_tool()` as before.

---

## Standalone MCP Server per Skill (optional)

For skills with `mcp_capable() == true`, the `trusty-mcp` crate (or a future `trusty-skill-server` binary) can expose each skill as its own MCP server:

```
trusty-metro-north --mcp --port 3460
# Exposes get_train_schedule + get_train_alerts as MCP tools on port 3460
```

Implementation: each skill crate's `main.rs` (or a thin wrapper) starts an axum server implementing the MCP SSE protocol, routing all tool calls through `skill.execute()`.

---

## `SearchSkills` Tool Updates

`SearchSkills` scans the registered skill registry (in-memory) instead of reading `docs/skills/*.md` files:

```rust
CoreTool::SearchSkills => {
    let query = args["query"].as_str().unwrap_or("");
    let results: Vec<_> = self.skills.iter()
        .filter(|s| {
            s.name().contains(query) || s.description().contains(query)
            || s.tools().iter().any(|t| t.description.contains(query))
        })
        .map(|s| format!("**{}**: {}", s.name(), s.description()))
        .collect();
    if results.is_empty() {
        "No matching skills found.".to_string()
    } else {
        results.join("\n")
    }
}
```

---

## Scripted Skill Discovery

`trusty-skill` provides `load_script_skills(dir: &Path) -> Vec<Arc<dyn Skill>>` that:
1. Scans `dir` for `*.md` files with `execute:` in the YAML frontmatter
2. Creates a `ScriptSkill` for each
3. Returns the vec to be appended to the skills list at startup

This means adding a new Python/shell skill is as simple as dropping a `.md` file in `docs/skills/` — no Cargo changes needed.

---

## Migration Plan

| Phase | Work | Crates Affected |
|-------|------|----------------|
| 1 | Create `trusty-skill` crate with `Skill` trait + `ScriptSkill` | new |
| 2 | Refactor `trusty-chat`: `CoreTool` enum, dynamic dispatch, skill registry | `trusty-chat` |
| 3 | Implement `MetroNorthSkill` in `trusty-metro-north` | `trusty-metro-north` |
| 4 | Implement `WeatherSkill` in `trusty-weather` | `trusty-weather` |
| 5 | Wire skills at startup in daemon, API, MCP | `trusty-daemon`, `trusty-api`, `trusty-mcp` |
| 6 | Load script skills from `docs/skills/*.md` at startup | `trusty-skill`, `trusty-daemon` |

All phases maintain backwards compatibility — existing tool behaviour is unchanged.
