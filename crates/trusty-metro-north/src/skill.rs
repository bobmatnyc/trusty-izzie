//! `MetroNorthSkill` — implements the `Skill` trait for Metro North tools.

use async_trait::async_trait;
use trusty_skill::{Skill, SkillTool, SkillToolCall};

pub struct MetroNorthSkill;

#[async_trait]
impl Skill for MetroNorthSkill {
    fn name(&self) -> &str {
        "metro-north"
    }

    fn description(&self) -> &str {
        "Real-time Metro North Railroad train schedules and service alerts (Hudson, New Haven, Harlem lines)"
    }

    fn tools(&self) -> Vec<SkillTool> {
        vec![
            SkillTool {
                name: "get_train_schedule".into(),
                description: "Fetch upcoming Metro North Railroad train departures between two stations using real-time MTA data".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "from_station": { "type": "string", "description": "Origin station name (e.g. \"Grand Central\", \"Stamford\", \"Hastings-on-Hudson\")" },
                        "to_station": { "type": "string", "description": "Destination station name" },
                        "count": { "type": "integer", "default": 5, "minimum": 1, "maximum": 20, "description": "Number of upcoming trains to return" }
                    },
                    "required": ["from_station", "to_station"]
                }),
            },
            SkillTool {
                name: "get_train_alerts".into(),
                description: "Fetch current Metro North Railroad service alerts and delays".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "line": { "type": "string", "description": "Optional line filter: New Haven, Harlem, Hudson, Pascack Valley, Port Jervis, New Canaan, Danbury, Waterbury" }
                    }
                }),
            },
        ]
    }

    fn system_prompt_contribution(&self) -> Option<String> {
        let path = std::path::Path::new("docs/skills/metro-north.md");
        std::fs::read_to_string(path).ok()
    }

    async fn execute(&self, call: &SkillToolCall) -> Option<anyhow::Result<String>> {
        match call.name.as_str() {
            "get_train_schedule" => Some(crate::get_train_schedule(&call.arguments).await),
            "get_train_alerts" => Some(crate::get_train_alerts(&call.arguments).await),
            _ => None,
        }
    }

    fn mcp_capable(&self) -> bool {
        true
    }
}
