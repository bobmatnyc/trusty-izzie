//! `WeatherSkill` — implements the `Skill` trait for weather tools.

use async_trait::async_trait;
use trusty_skill::{Skill, SkillTool, SkillToolCall};

pub struct WeatherSkill;

#[async_trait]
impl Skill for WeatherSkill {
    fn name(&self) -> &str {
        "weather"
    }

    fn description(&self) -> &str {
        "Weather forecasts (Open-Meteo) and active NWS severe weather alerts for US locations"
    }

    fn tools(&self) -> Vec<SkillTool> {
        vec![
            SkillTool {
                name: "get_weather".into(),
                description: "Get weather forecast for a location. Defaults to user's home location if none specified.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string", "description": "City name, address, or \"home\" for user's location" },
                        "days": { "type": "integer", "default": 3, "minimum": 1, "maximum": 7, "description": "Forecast days (1-7)" }
                    }
                }),
            },
            SkillTool {
                name: "get_weather_alerts".into(),
                description: "Get active NWS severe weather alerts for a US location".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string", "description": "City name or address (US only)" }
                    }
                }),
            },
        ]
    }

    fn system_prompt_contribution(&self) -> Option<String> {
        let path = std::path::Path::new("docs/skills/weather.md");
        std::fs::read_to_string(path).ok()
    }

    async fn execute(&self, call: &SkillToolCall) -> Option<anyhow::Result<String>> {
        match call.name.as_str() {
            "get_weather" => Some(crate::get_weather(&call.arguments).await),
            "get_weather_alerts" => Some(crate::get_weather_alerts(&call.arguments).await),
            _ => None,
        }
    }

    fn mcp_capable(&self) -> bool {
        false
    }
}
