//! VIP email alert (stub — requires Gmail sync to be fully wired).

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use trusty_core::error::TrustyError;
use trusty_models::{EventType, QueuedEvent};
use trusty_store::Store;

use super::{DispatchResult, EventHandler};

pub struct VipEmailCheckHandler;

#[async_trait]
impl EventHandler for VipEmailCheckHandler {
    fn event_type(&self) -> EventType {
        EventType::VipEmailCheck
    }

    async fn handle(
        &self,
        _event: &QueuedEvent,
        _store: &Arc<Store>,
    ) -> Result<DispatchResult, TrustyError> {
        info!("VipEmailCheck: stub handler — Gmail sync not yet wired");
        Ok(DispatchResult::Done)
    }
}
