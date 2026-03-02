use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventPayload, EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct ReminderHandler;

#[async_trait]
impl EventHandler for ReminderHandler {
    fn event_type(&self) -> EventType {
        EventType::Reminder
    }

    async fn handle(
        &self,
        event: &QueuedEvent,
        _store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        if let EventPayload::Reminder {
            message, subtitle, ..
        } = &event.payload
        {
            #[cfg(target_os = "macos")]
            {
                let subtitle_part = subtitle
                    .as_deref()
                    .map(|s| format!(r#" subtitle "{}""#, s.replace('"', r#"\""#)))
                    .unwrap_or_default();
                let script = format!(
                    r#"display notification "{}" with title "trusty-izzie"{}"#,
                    message.replace('"', r#"\""#),
                    subtitle_part,
                );
                let status = std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .status()
                    .map_err(TrustyError::Io)?;
                if !status.success() {
                    return Err(TrustyError::Storage("osascript failed".to_string()));
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                info!("Reminder (non-macOS): {}", message);
            }
            info!("Reminder fired: {}", message);
        }
        Ok(DispatchResult::Done)
    }
}
