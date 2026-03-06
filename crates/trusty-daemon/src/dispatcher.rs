use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};
use trusty_core::error::TrustyError;
use trusty_store::Store;

use std::path::PathBuf;

use crate::handlers::{
    AgentRunHandler, CalendarRefreshHandler, ContactsSyncHandler, DispatchResult, EmailSyncHandler,
    EntityExtractionHandler, EveningBriefingHandler, EventHandler, FollowUpHandler,
    MemoryDecayHandler, MessageInterruptCheckHandler, MessagesSyncHandler, MorningBriefingHandler,
    NeedsReauthHandler, RelationshipNudgeHandler, ReminderHandler, VipEmailCheckHandler,
    WatchCheckHandler, WeeklyDigestHandler, WhatsAppSyncHandler,
};

pub struct EventDispatcher {
    store: Arc<Store>,
    handlers: HashMap<String, Box<dyn EventHandler>>,
}

/// Exponential backoff: base 30s, capped at 32 minutes.
fn exponential_backoff(attempts: i64) -> i64 {
    let base = 30i64;
    let cap = 32 * 60i64;
    let delay = base * 2i64.saturating_pow(attempts.saturating_sub(1) as u32);
    chrono::Utc::now().timestamp() + delay.min(cap)
}

impl EventDispatcher {
    pub fn new(store: Arc<Store>) -> Self {
        Self::new_with_agents(
            store,
            PathBuf::from("docs/agents"),
            "https://openrouter.ai/api/v1".to_string(),
            std::env::var("OPENROUTER_API_KEY").unwrap_or_default(),
        )
    }

    pub fn new_with_agents(
        store: Arc<Store>,
        agents_dir: PathBuf,
        openrouter_base: String,
        openrouter_api_key: String,
    ) -> Self {
        let mut handlers: HashMap<String, Box<dyn EventHandler>> = HashMap::new();

        for h in Self::all_handlers(agents_dir, openrouter_base, openrouter_api_key) {
            handlers.insert(h.event_type().as_str().to_string(), h);
        }

        Self { store, handlers }
    }

    fn all_handlers(
        agents_dir: PathBuf,
        openrouter_base: String,
        openrouter_api_key: String,
    ) -> Vec<Box<dyn EventHandler>> {
        vec![
            Box::new(NeedsReauthHandler),
            Box::new(ReminderHandler),
            Box::new(EmailSyncHandler),
            Box::new(ContactsSyncHandler),
            Box::new(MessagesSyncHandler),
            Box::new(WhatsAppSyncHandler),
            Box::new(EntityExtractionHandler),
            Box::new(MemoryDecayHandler),
            Box::new(CalendarRefreshHandler),
            Box::new(AgentRunHandler::new(
                agents_dir,
                openrouter_base.clone(),
                openrouter_api_key.clone(),
            )),
            Box::new(MorningBriefingHandler::new(
                openrouter_base.clone(),
                openrouter_api_key.clone(),
            )),
            Box::new(EveningBriefingHandler::new(
                openrouter_base.clone(),
                openrouter_api_key.clone(),
            )),
            Box::new(WeeklyDigestHandler::new(
                openrouter_base.clone(),
                openrouter_api_key.clone(),
            )),
            Box::new(WatchCheckHandler::new(openrouter_base, openrouter_api_key)),
            Box::new(FollowUpHandler),
            Box::new(RelationshipNudgeHandler),
            Box::new(VipEmailCheckHandler),
            Box::new(MessageInterruptCheckHandler),
        ]
    }

    /// Poll for claimable events and dispatch them until the queue is empty.
    pub async fn tick(&self) -> Result<(), TrustyError> {
        loop {
            let sqlite = self.store.sqlite.clone();
            let event_opt = tokio::task::spawn_blocking(move || sqlite.claim_next_event())
                .await
                .map_err(|e| TrustyError::Storage(e.to_string()))?
                .map_err(|e| TrustyError::Storage(e.to_string()))?;

            let event = match event_opt {
                None => break,
                Some(e) => e,
            };

            let id_str = event.id.to_string();
            let event_type_str = event.event_type.as_str().to_string();

            match self.handlers.get(&event_type_str) {
                None => {
                    warn!("No handler registered for event type: {}", event_type_str);
                    let sqlite = self.store.sqlite.clone();
                    tokio::task::spawn_blocking(move || {
                        sqlite.fail_event(&id_str, "no handler registered", None)
                    })
                    .await
                    .map_err(|e| TrustyError::Storage(e.to_string()))?
                    .map_err(|e| TrustyError::Storage(e.to_string()))?;
                }
                Some(handler) => {
                    info!("Dispatching {} event {}", event_type_str, id_str);
                    match handler.handle(&event, &self.store).await {
                        Ok(DispatchResult::Done) => {
                            info!("Event {} completed", id_str);
                            let sqlite = self.store.sqlite.clone();
                            let id = id_str.clone();
                            tokio::task::spawn_blocking(move || sqlite.complete_event(&id))
                                .await
                                .map_err(|e| TrustyError::Storage(e.to_string()))?
                                .map_err(|e| TrustyError::Storage(e.to_string()))?;
                        }
                        Ok(DispatchResult::Chain(children)) => {
                            info!(
                                "Event {} completed, chaining {} child(ren)",
                                id_str,
                                children.len()
                            );
                            let sqlite = self.store.sqlite.clone();
                            let id = id_str.clone();
                            tokio::task::spawn_blocking(move || {
                                sqlite.complete_event(&id)?;
                                for (child_type, child_payload, child_sched) in children {
                                    sqlite.enqueue_event(
                                        &child_type,
                                        &child_payload,
                                        child_sched,
                                        child_type.default_priority(),
                                        child_type.default_max_retries(),
                                        "system",
                                        Some(id.as_str()),
                                    )?;
                                }
                                Ok::<_, anyhow::Error>(())
                            })
                            .await
                            .map_err(|e| TrustyError::Storage(e.to_string()))?
                            .map_err(|e| TrustyError::Storage(e.to_string()))?;
                        }
                        Err(e) => {
                            error!("Event {} failed: {}", id_str, e);
                            let retry_after = if event.attempts < event.max_retries {
                                Some(exponential_backoff(event.attempts))
                            } else {
                                None
                            };
                            let sqlite = self.store.sqlite.clone();
                            let id = id_str.clone();
                            let err_str = e.to_string();
                            tokio::task::spawn_blocking(move || {
                                sqlite.fail_event(&id, &err_str, retry_after)
                            })
                            .await
                            .map_err(|e| TrustyError::Storage(e.to_string()))?
                            .map_err(|e| TrustyError::Storage(e.to_string()))?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
