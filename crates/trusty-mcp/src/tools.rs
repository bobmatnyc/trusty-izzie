//! Tool definitions (schemas) and dispatch to ChatEngine.

use anyhow::Result;
use serde_json::{json, Value};
use trusty_chat::engine::ChatEngine;
use trusty_chat::tools::ToolName;

use crate::protocol::Tool;

/// All MCP tools exposed by trusty-izzie.
pub fn all_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "search_contacts",
            description: "Search your macOS AddressBook contacts by name, email, or phone number",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Name, email, or phone to search for" },
                    "limit": { "type": "integer", "default": 10, "minimum": 1, "maximum": 50 }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "get_calendar_events",
            description: "Fetch upcoming Google Calendar events across all connected accounts",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "days": { "type": "integer", "default": 7, "minimum": 1, "maximum": 30,
                              "description": "How many days ahead to look" },
                    "account_email": { "type": "string",
                                       "description": "Specific Google account email (omit for all accounts)" }
                }
            }),
        },
        Tool {
            name: "create_calendar_event",
            description: "Create a new event on a connected Google Calendar",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_email": { "type": "string", "description": "Google account to create event on" },
                    "title": { "type": "string" },
                    "start_datetime": { "type": "string", "description": "ISO 8601 datetime, e.g. 2026-03-15T14:00:00-05:00" },
                    "end_datetime": { "type": "string", "description": "ISO 8601 datetime" },
                    "description": { "type": "string" },
                    "attendees": { "type": "array", "items": { "type": "string" },
                                   "description": "List of attendee email addresses" }
                },
                "required": ["account_email", "title", "start_datetime", "end_datetime"]
            }),
        },
        Tool {
            name: "get_tasks_bulk",
            description: "Fetch all task lists and all incomplete tasks for a Google account in a single call",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_email": { "type": "string", "description": "Google account (defaults to primary)" }
                }
            }),
        },
        Tool {
            name: "get_tasks",
            description: "Fetch tasks from a Google Task list",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_email": { "type": "string", "description": "Google account email" },
                    "task_list_id": { "type": "string", "description": "Task list ID (omit for default list)" }
                },
                "required": ["account_email"]
            }),
        },
        Tool {
            name: "search_memories",
            description: "Semantic search over Izzie's stored memories (facts, notes, past observations)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language query" },
                    "limit": { "type": "integer", "default": 10, "minimum": 1, "maximum": 50 }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "search_entities",
            description: "Search the knowledge graph for people, organizations, and topics",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Name or description to search" },
                    "entity_type": { "type": "string", "enum": ["person", "organization", "topic", "project"],
                                     "description": "Filter by entity type (optional)" },
                    "limit": { "type": "integer", "default": 10, "minimum": 1, "maximum": 50 }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "get_entity_relationships",
            description: "Get all known relationships for a named entity (who they work with, etc.)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Entity name to look up" }
                },
                "required": ["name"]
            }),
        },
        Tool {
            name: "get_context",
            description: "Get full personal context about a person or topic: memories, entities, and upcoming calendar",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Person name, email, or topic to contextualize" }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "list_accounts",
            description: "List connected Google accounts and their capabilities (calendar, tasks, email)",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "schedule_event",
            description: "Schedule a future reminder or system event",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "event_type": { "type": "string", "enum": ["reminder", "email_sync", "calendar_refresh"],
                                    "description": "Type of event to schedule" },
                    "scheduled_at": { "type": "string", "description": "ISO 8601 datetime in the future" },
                    "message": { "type": "string", "description": "Reminder message (for reminder events)" }
                },
                "required": ["event_type", "scheduled_at"]
            }),
        },
        Tool {
            name: "list_open_loops",
            description: "List pending follow-up items and open loops tracked by Izzie",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "default": 20 }
                }
            }),
        },
        Tool {
            name: "get_preferences",
            description: "Get the user's Izzie preferences (proactive features, notification settings)",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
    ]
}

/// Dispatch a tool call by name to the ChatEngine.
pub async fn dispatch(engine: &ChatEngine, name: &str, arguments: &Value) -> Result<String> {
    match name {
        "search_contacts" => {
            engine
                .execute_tool(&ToolName::SearchContacts, arguments)
                .await
        }
        "get_calendar_events" => {
            engine
                .execute_tool(&ToolName::GetCalendarEvents, arguments)
                .await
        }
        "create_calendar_event" => {
            engine
                .execute_tool(&ToolName::CreateCalendarEvent, arguments)
                .await
        }
        "get_tasks_bulk" => {
            engine
                .execute_tool(&ToolName::GetTasksBulk, arguments)
                .await
        }
        "get_tasks" => engine.execute_tool(&ToolName::GetTasks, arguments).await,
        "search_memories" => {
            engine
                .execute_tool(&ToolName::SearchMemories, arguments)
                .await
        }
        "search_entities" => {
            engine
                .execute_tool(&ToolName::SearchEntities, arguments)
                .await
        }
        "get_entity_relationships" => {
            engine
                .execute_tool(&ToolName::GetEntityRelationships, arguments)
                .await
        }
        "get_context" => tool_get_context(engine, arguments).await,
        "list_accounts" => {
            engine
                .execute_tool(&ToolName::ListAccounts, arguments)
                .await
        }
        "schedule_event" => {
            engine
                .execute_tool(&ToolName::ScheduleEvent, arguments)
                .await
        }
        "list_open_loops" => {
            engine
                .execute_tool(&ToolName::ListOpenLoops, arguments)
                .await
        }
        "get_preferences" => {
            engine
                .execute_tool(&ToolName::GetPreferences, arguments)
                .await
        }
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    }
}

/// Composite context tool: memories + entities + calendar for a query.
async fn tool_get_context(engine: &ChatEngine, arguments: &Value) -> Result<String> {
    let query = arguments["query"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

    let memories = engine
        .execute_tool(
            &ToolName::SearchMemories,
            &json!({"query": query, "limit": 5}),
        )
        .await
        .unwrap_or_else(|e| format!("(unavailable: {e})"));

    let entities = engine
        .execute_tool(
            &ToolName::SearchEntities,
            &json!({"query": query, "limit": 5}),
        )
        .await
        .unwrap_or_else(|e| format!("(unavailable: {e})"));

    let calendar = engine
        .execute_tool(&ToolName::GetCalendarEvents, &json!({"days": 14}))
        .await
        .unwrap_or_else(|e| format!("(unavailable: {e})"));

    Ok(format!(
        "## Context for \"{query}\"\n\n### Memories\n{memories}\n\n### Known Entities\n{entities}\n\n### Upcoming Calendar\n{calendar}"
    ))
}
