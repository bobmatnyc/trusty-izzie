use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct MemoryDecayHandler;

fn next_midnight() -> i64 {
    let now = Utc::now();
    let tomorrow = (now + Duration::days(1))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    Utc.from_utc_datetime(&tomorrow).timestamp()
}

#[async_trait]
impl EventHandler for MemoryDecayHandler {
    fn event_type(&self) -> EventType {
        EventType::MemoryDecay
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        _store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        if let EventPayload::MemoryDecay { min_age_days } = &event.payload {
            info!("MemoryDecay stub: min_age_days={:?}", min_age_days);
            // TODO: call trusty_memory::apply_decay() when implemented
        }
        Ok(DispatchResult::Chain(vec![(
            EventType::MemoryDecay,
            EventPayload::MemoryDecay { min_age_days: None },
            next_midnight(),
        )]))
    }
}
