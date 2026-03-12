//! Proactive train delay check — sends Telegram alert if Hudson line has disruptions.
//!
//! Scheduled twice daily: 7:00 AM (before AM commute) and 4:30 PM (before PM commute).
//! Only fires if trusty_metro_north::get_train_alerts reports active alerts.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};
use trusty_core::error::TrustyError;
use trusty_metro_north;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::scheduling::next_time_of_day_ts;
use crate::telegram_push::send_telegram_push;

pub struct TrainDelayCheckHandler;

#[async_trait]
impl EventHandler for TrainDelayCheckHandler {
    fn event_type(&self) -> EventType {
        EventType::TrainDelayCheck
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        let args = serde_json::json!({ "line": "Hudson" });
        match trusty_metro_north::get_train_alerts(&args).await {
            Ok(alert_text) => {
                if alert_text.contains("No active service alerts") {
                    info!("No Hudson line alerts — skipping notification");
                } else {
                    let message = format!("Metro North Alert\n\n{alert_text}");
                    send_telegram_push(&store.sqlite, &message).await?;
                    info!("Sent train delay notification");
                }
            }
            Err(e) => {
                warn!("Failed to fetch train alerts: {e}");
            }
        }

        Ok(schedule_next_check())
    }
}

fn schedule_next_check() -> DispatchResult {
    // Alternate between the two commute windows.
    // next_time_of_day_ts returns the soonest future occurrence of that wall-clock time,
    // so whichever of 07:00 or 16:30 is still in the future today wins; if both have
    // passed it returns 07:00 tomorrow.
    let am_ts = next_time_of_day_ts(7, 0);
    let pm_ts = next_time_of_day_ts(16, 30);
    let next_ts = am_ts.min(pm_ts);

    DispatchResult::Chain(vec![(
        EventType::TrainDelayCheck,
        EventPayload::TrainDelayCheck {},
        next_ts,
    )])
}
