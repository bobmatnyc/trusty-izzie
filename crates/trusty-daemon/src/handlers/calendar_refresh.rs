use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct CalendarRefreshHandler;

#[async_trait]
impl EventHandler for CalendarRefreshHandler {
    fn event_type(&self) -> EventType {
        EventType::CalendarRefresh
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        _store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        if let EventPayload::CalendarRefresh { lookahead_days } = &event.payload {
            info!("CalendarRefresh stub: lookahead_days={}", lookahead_days);
            // TODO: call calendar API when implemented
        }
        // Re-schedule in 30 minutes.
        let next_at = chrono::Utc::now().timestamp() + 1800;
        Ok(DispatchResult::Chain(vec![(
            EventType::CalendarRefresh,
            EventPayload::CalendarRefresh { lookahead_days: 7 },
            next_at,
        )]))
    }
}
