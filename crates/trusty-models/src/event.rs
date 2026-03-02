use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Reminder,
    EmailSync,
    EntityExtraction,
    MemoryDecay,
    CalendarRefresh,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::Reminder => "reminder",
            EventType::EmailSync => "email_sync",
            EventType::EntityExtraction => "entity_extraction",
            EventType::MemoryDecay => "memory_decay",
            EventType::CalendarRefresh => "calendar_refresh",
        }
    }

    pub fn default_priority(&self) -> i64 {
        match self {
            EventType::Reminder => 2,
            EventType::EmailSync => 4,
            EventType::EntityExtraction => 5,
            EventType::CalendarRefresh => 6,
            EventType::MemoryDecay => 8,
        }
    }

    pub fn default_max_retries(&self) -> i64 {
        match self {
            EventType::Reminder => 1,
            EventType::EmailSync => 3,
            EventType::EntityExtraction => 5,
            EventType::CalendarRefresh => 3,
            EventType::MemoryDecay => 2,
        }
    }
}

impl std::str::FromStr for EventType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "reminder" => Ok(EventType::Reminder),
            "email_sync" => Ok(EventType::EmailSync),
            "entity_extraction" => Ok(EventType::EntityExtraction),
            "memory_decay" => Ok(EventType::MemoryDecay),
            "calendar_refresh" => Ok(EventType::CalendarRefresh),
            _ => Err(format!("unknown event type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    Reminder {
        message: String,
        subtitle: Option<String>,
        url: Option<String>,
    },
    EmailSync {
        force: bool,
    },
    EntityExtraction {
        message_ids: Vec<String>,
        source_event_id: Option<String>,
    },
    MemoryDecay {
        min_age_days: Option<u32>,
    },
    CalendarRefresh {
        lookahead_days: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl EventStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventStatus::Pending => "pending",
            EventStatus::Running => "running",
            EventStatus::Done => "done",
            EventStatus::Failed => "failed",
            EventStatus::Cancelled => "cancelled",
        }
    }
}

impl std::str::FromStr for EventStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(EventStatus::Pending),
            "running" => Ok(EventStatus::Running),
            "done" => Ok(EventStatus::Done),
            "failed" => Ok(EventStatus::Failed),
            "cancelled" => Ok(EventStatus::Cancelled),
            _ => Err(format!("unknown event status: {}", s)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueuedEvent {
    pub id: Uuid,
    pub event_type: EventType,
    pub payload: EventPayload,
    pub status: EventStatus,
    pub priority: i64,
    pub scheduled_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub attempts: i64,
    pub max_retries: i64,
    pub retry_after: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub source: String,
    pub parent_event_id: Option<Uuid>,
}
