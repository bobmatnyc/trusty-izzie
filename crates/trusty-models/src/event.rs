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
    NeedsReauth,
    AgentRun,
    ContactsSync,
    MessagesSync,
    WhatsAppSync,
    MorningBriefing,
    EveningBriefing,
    WeeklyDigest,
    VipEmailCheck,
    FollowUp,
    RelationshipNudge,
    WatchCheck,
    MessageInterruptCheck,
    TrainDelayCheck,
    WeatherCheck,
    StyleTraining,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::Reminder => "reminder",
            EventType::EmailSync => "email_sync",
            EventType::EntityExtraction => "entity_extraction",
            EventType::MemoryDecay => "memory_decay",
            EventType::CalendarRefresh => "calendar_refresh",
            EventType::NeedsReauth => "needs_reauth",
            EventType::AgentRun => "agent_run",
            EventType::ContactsSync => "contacts_sync",
            EventType::MessagesSync => "messages_sync",
            EventType::WhatsAppSync => "whatsapp_sync",
            EventType::MorningBriefing => "morning_briefing",
            EventType::EveningBriefing => "evening_briefing",
            EventType::WeeklyDigest => "weekly_digest",
            EventType::VipEmailCheck => "vip_email_check",
            EventType::FollowUp => "follow_up",
            EventType::RelationshipNudge => "relationship_nudge",
            EventType::WatchCheck => "watch_check",
            EventType::MessageInterruptCheck => "message_interrupt_check",
            EventType::TrainDelayCheck => "train_delay_check",
            EventType::WeatherCheck => "weather_check",
            EventType::StyleTraining => "style_training",
        }
    }

    pub fn default_priority(&self) -> i64 {
        match self {
            EventType::NeedsReauth => 1,
            EventType::Reminder => 2,
            EventType::VipEmailCheck => 2,
            EventType::AgentRun => 3,
            EventType::MorningBriefing => 3,
            EventType::EveningBriefing => 3,
            EventType::WeeklyDigest => 3,
            EventType::FollowUp => 3,
            EventType::EmailSync => 4,
            EventType::EntityExtraction => 5,
            EventType::MessagesSync => 5,
            EventType::WhatsAppSync => 5,
            EventType::CalendarRefresh => 6,
            EventType::ContactsSync => 6,
            EventType::RelationshipNudge => 4,
            EventType::WatchCheck => 4,
            EventType::MessageInterruptCheck => 4,
            EventType::TrainDelayCheck => 3,
            EventType::WeatherCheck => 3,
            EventType::MemoryDecay => 8,
            EventType::StyleTraining => 5,
        }
    }

    pub fn default_max_retries(&self) -> i64 {
        match self {
            EventType::NeedsReauth => 1,
            EventType::Reminder => 1,
            EventType::AgentRun => 1,
            EventType::MorningBriefing => 1,
            EventType::EveningBriefing => 1,
            EventType::WeeklyDigest => 1,
            EventType::FollowUp => 1,
            EventType::RelationshipNudge => 1,
            EventType::WatchCheck => 1,
            EventType::MessageInterruptCheck => 2,
            EventType::TrainDelayCheck => 1,
            EventType::WeatherCheck => 1,
            EventType::VipEmailCheck => 2,
            EventType::EmailSync => 3,
            EventType::CalendarRefresh => 3,
            EventType::MessagesSync => 3,
            EventType::WhatsAppSync => 3,
            EventType::MemoryDecay => 2,
            EventType::ContactsSync => 2,
            EventType::EntityExtraction => 5,
            EventType::StyleTraining => 1,
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
            "needs_reauth" => Ok(EventType::NeedsReauth),
            "agent_run" => Ok(EventType::AgentRun),
            "contacts_sync" => Ok(EventType::ContactsSync),
            "messages_sync" => Ok(EventType::MessagesSync),
            "whatsapp_sync" => Ok(EventType::WhatsAppSync),
            "morning_briefing" => Ok(EventType::MorningBriefing),
            "evening_briefing" => Ok(EventType::EveningBriefing),
            "weekly_digest" => Ok(EventType::WeeklyDigest),
            "vip_email_check" => Ok(EventType::VipEmailCheck),
            "follow_up" => Ok(EventType::FollowUp),
            "relationship_nudge" => Ok(EventType::RelationshipNudge),
            "watch_check" => Ok(EventType::WatchCheck),
            "message_interrupt_check" => Ok(EventType::MessageInterruptCheck),
            "train_delay_check" => Ok(EventType::TrainDelayCheck),
            "weather_check" => Ok(EventType::WeatherCheck),
            "style_training" => Ok(EventType::StyleTraining),
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
    NeedsReauth {
        /// Reason code: "no_token" | "token_expired" | "auth_error"
        reason: String,
    },
    AgentRun {
        /// Matches the filename stem in docs/agents/{agent_name}.md
        agent_name: String,
        task_description: String,
        /// Optional extra context injected into the agent prompt
        context: Option<String>,
    },
    ContactsSync {
        force: bool,
    },
    MessagesSync {
        force: bool,
    },
    WhatsAppSync {
        export_path: Option<String>,
        force: bool,
    },
    MorningBriefing {},
    EveningBriefing {},
    WeeklyDigest {},
    VipEmailCheck {
        email: String,
    },
    FollowUp {
        open_loop_id: String,
        description: String,
    },
    RelationshipNudge {
        email: String,
        name: String,
        last_contact_days: u32,
    },
    WatchCheck {
        subscription_id: String,
        topic: String,
    },
    MessageInterruptCheck {},
    TrainDelayCheck {},
    WeatherCheck {},
    StyleTraining {},
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
