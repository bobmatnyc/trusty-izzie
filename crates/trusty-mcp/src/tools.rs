//! Tool definitions (schemas) and dispatch to ChatEngine.

use anyhow::Result;
use serde_json::{json, Value};
use trusty_chat::engine::ChatEngine;
use trusty_chat::SessionManager;

use crate::protocol::Tool;

/// All MCP tools exposed by trusty-izzie.
pub fn all_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "chat",
            description: "Ask Izzie an open-ended question in natural language. She has full access to your contacts, calendar, tasks, emails, memories, and knowledge graph, and will use whichever tools are needed to answer. Use this when the query doesn't fit a more specific tool.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Your question or request in natural language"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional session ID to continue a previous conversation thread"
                    }
                },
                "required": ["message"]
            }),
        },
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
            name: "complete_task",
            description: "Mark a Google Task as complete. Use get_tasks_bulk first to find the task_list_id and task_id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_email": { "type": "string", "description": "Google account email" },
                    "task_list_id": { "type": "string", "description": "Task list ID (from get_tasks_bulk)" },
                    "task_id": { "type": "string", "description": "Task ID to mark complete (from get_tasks_bulk)" }
                },
                "required": ["account_email", "task_list_id", "task_id"]
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
        Tool {
            name: "get_train_schedule",
            description: "Fetch upcoming Metro North Railroad train departures between two stations using real-time MTA data",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "from_station": { "type": "string", "description": "Origin station name (e.g. \"Grand Central\", \"Stamford\", \"Greenwich\")" },
                    "to_station": { "type": "string", "description": "Destination station name" },
                    "count": { "type": "integer", "default": 5, "minimum": 1, "maximum": 20,
                               "description": "Number of upcoming trains to return" }
                },
                "required": ["from_station", "to_station"]
            }),
        },
        Tool {
            name: "get_train_alerts",
            description: "Fetch current Metro North Railroad service alerts and delays",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "line": { "type": "string",
                              "description": "Optional line filter: New Haven, Harlem, Hudson, Pascack Valley, Port Jervis, New Canaan, Danbury, Waterbury" }
                }
            }),
        },
    ]
}

/// Dispatch a tool call by name to the ChatEngine.
pub async fn dispatch(engine: &ChatEngine, name: &str, arguments: &Value) -> Result<String> {
    match name {
        "chat" => tool_chat(engine, arguments).await,
        "get_context" => tool_get_context(engine, arguments).await,
        other => engine.execute_tool_by_name(other, arguments).await,
    }
}

/// Open-ended chat: runs a full Izzie turn (with tool calls) and returns the reply.
///
/// The `session_id` argument is accepted but currently unused — each MCP call
/// gets a fresh ephemeral session. Stateful multi-turn support can be added later
/// by persisting sessions keyed by `session_id`.
async fn tool_chat(engine: &ChatEngine, arguments: &Value) -> Result<String> {
    let message = arguments["message"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: message"))?;

    // Ephemeral session — no persistence across MCP calls for now.
    let mut session = SessionManager::new_session("mcp");

    let response = engine.chat(&mut session, message).await?;

    // Return the plain text reply; ignore any trailing tool calls.
    Ok(response.reply)
}

/// Composite context tool: memories + entities + calendar for a query.
async fn tool_get_context(engine: &ChatEngine, arguments: &Value) -> Result<String> {
    let query = arguments["query"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

    let memories = engine
        .execute_tool_by_name("search_memories", &json!({"query": query, "limit": 5}))
        .await
        .unwrap_or_else(|e| format!("(unavailable: {e})"));

    let entities = engine
        .execute_tool_by_name("search_entities", &json!({"query": query, "limit": 5}))
        .await
        .unwrap_or_else(|e| format!("(unavailable: {e})"));

    let calendar = engine
        .execute_tool_by_name("get_calendar_events", &json!({"days": 14}))
        .await
        .unwrap_or_else(|e| format!("(unavailable: {e})"));

    Ok(format!(
        "## Context for \"{query}\"\n\n### Memories\n{memories}\n\n### Known Entities\n{entities}\n\n### Upcoming Calendar\n{calendar}"
    ))
}
