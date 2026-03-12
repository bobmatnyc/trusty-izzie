//! Proactive weather check -- sends Telegram alert for severe weather or
//! significant forecast events (heavy rain, snow, extreme temps, high winds).
//!
//! Runs daily at 7:30 AM (after train delay check, before commute).
//! Also triggers on any active NWS Severe/Extreme alert.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};
use crate::scheduling::next_time_of_day_ts;
use crate::telegram_push::send_telegram_push;

pub struct WeatherCheckHandler;

#[async_trait]
impl EventHandler for WeatherCheckHandler {
    fn event_type(&self) -> EventType {
        EventType::WeatherCheck
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        match trusty_weather::proactive_check(
            trusty_weather::geocode::DEFAULT_LAT,
            trusty_weather::geocode::DEFAULT_LON,
            trusty_weather::geocode::DEFAULT_LOCATION,
        )
        .await
        {
            Some(msg) => {
                let message =
                    format!("Weather Alert\n\n{msg}\n\nUse 'get weather' for full forecast.");
                send_telegram_push(&store.sqlite, &message).await?;
                info!("Sent proactive weather alert");
            }
            None => {
                info!("No significant weather -- skipping notification");
            }
        }

        Ok(schedule_next_check())
    }
}

fn schedule_next_check() -> DispatchResult {
    DispatchResult::Chain(vec![(
        EventType::WeatherCheck,
        EventPayload::WeatherCheck {},
        next_time_of_day_ts(7, 30),
    )])
}
