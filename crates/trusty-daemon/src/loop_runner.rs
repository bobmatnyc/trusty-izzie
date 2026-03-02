//! The main daemon polling loop.

use std::time::Duration;
use tokio::time;
use tracing::info;
use trusty_core::error::TrustyError;

use crate::dispatcher::EventDispatcher;

/// Drives the recurring event-dispatch loop until a shutdown signal is received.
pub struct DaemonLoop;

impl DaemonLoop {
    pub fn new() -> Self {
        DaemonLoop
    }

    /// Run the dispatch loop, polling every 10 seconds.
    ///
    /// The loop exits cleanly when `shutdown` resolves.
    pub async fn run(
        &self,
        dispatcher: &EventDispatcher,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> Result<(), TrustyError> {
        let mut ticker = time::interval(Duration::from_secs(10));
        tokio::pin!(shutdown);

        info!("Daemon loop started, polling every 10s");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = dispatcher.tick().await {
                        tracing::error!(error = %e, "dispatcher tick failed");
                    }
                }
                _ = &mut shutdown => {
                    info!("Daemon received shutdown signal");
                    break;
                }
            }
        }
        Ok(())
    }
}

impl Default for DaemonLoop {
    fn default() -> Self {
        Self::new()
    }
}
