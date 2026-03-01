//! The main daemon polling loop.

use anyhow::Result;
use std::time::Duration;
use tokio::time;
use tracing::info;

use trusty_models::config::DaemonConfig;

/// Runs the recurring email sync loop until a shutdown signal is received.
pub struct DaemonLoop {
    config: DaemonConfig,
}

impl DaemonLoop {
    /// Construct with daemon configuration.
    pub fn new(config: DaemonConfig) -> Self {
        Self { config }
    }

    /// Run the loop, calling `on_tick` at each interval.
    ///
    /// The loop exits cleanly when `shutdown` resolves.
    pub async fn run(
        &self,
        mut on_tick: impl AsyncFnMut() -> Result<()>,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> Result<()> {
        let interval = Duration::from_secs(self.config.email_sync_interval_secs);
        let mut ticker = time::interval(interval);

        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    info!("daemon tick: running email sync");
                    if let Err(e) = on_tick().await {
                        tracing::error!(error = %e, "sync tick failed");
                    }
                }
                _ = &mut shutdown => {
                    info!("daemon shutdown signal received");
                    break;
                }
            }
        }

        Ok(())
    }
}
