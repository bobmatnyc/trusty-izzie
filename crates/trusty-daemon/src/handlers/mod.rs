use async_trait::async_trait;
use std::sync::Arc;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

pub mod agent_run;
pub mod calendar_refresh;
pub mod contacts_sync;
pub mod email_sync;
pub mod entity_extraction;
pub mod evening_briefing;
pub mod follow_up;
pub mod memory_decay;
pub mod message_interrupt_check;
pub mod messages_sync;
pub mod morning_briefing;
pub mod needs_reauth;
pub mod relationship_nudge;
pub mod reminder;
pub mod vip_email_check;
pub mod watch_check;
pub mod weekly_digest;
pub mod whatsapp_sync;

pub use agent_run::AgentRunHandler;
pub use calendar_refresh::CalendarRefreshHandler;
pub use contacts_sync::ContactsSyncHandler;
pub use email_sync::EmailSyncHandler;
pub use entity_extraction::EntityExtractionHandler;
pub use evening_briefing::EveningBriefingHandler;
pub use follow_up::FollowUpHandler;
pub use memory_decay::MemoryDecayHandler;
pub use message_interrupt_check::MessageInterruptCheckHandler;
pub use messages_sync::MessagesSyncHandler;
pub use morning_briefing::MorningBriefingHandler;
pub use needs_reauth::NeedsReauthHandler;
pub use relationship_nudge::RelationshipNudgeHandler;
pub use reminder::ReminderHandler;
pub use vip_email_check::VipEmailCheckHandler;
pub use watch_check::WatchCheckHandler;
pub use weekly_digest::WeeklyDigestHandler;
pub use whatsapp_sync::WhatsAppSyncHandler;

/// The outcome of dispatching an event.
#[derive(Debug)]
pub enum DispatchResult {
    /// Event completed; nothing to chain.
    Done,
    /// Event completed; enqueue these children. Tuple: (type, payload, scheduled_at unix ts).
    Chain(Vec<(EventType, EventPayload, i64)>),
}

/// Trait implemented by each concrete event handler.
#[async_trait]
pub trait EventHandler: Send + Sync {
    fn event_type(&self) -> EventType;
    async fn handle(
        &self,
        event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError>;
}
