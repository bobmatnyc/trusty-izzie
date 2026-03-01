//! Unix domain socket IPC server for daemon control messages.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Messages the CLI can send to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonCommand {
    /// Request an immediate sync cycle.
    Sync { force: bool },
    /// Request the daemon's current status.
    Status,
    /// Instruct the daemon to shut down gracefully.
    Shutdown,
}

/// Responses the daemon sends back over the IPC socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonResponse {
    /// The command was accepted and is being processed.
    Ok,
    /// A status snapshot.
    Status {
        /// Whether the daemon is currently syncing.
        syncing: bool,
        /// Timestamp of last successful sync (ISO 8601).
        last_sync: Option<String>,
        /// Number of messages processed in the last sync.
        last_message_count: u32,
    },
    /// An error occurred while processing the command.
    Error { message: String },
}

/// Listens on the configured IPC socket path for incoming commands.
#[allow(dead_code)]
pub struct IpcServer {
    socket_path: String,
}

impl IpcServer {
    /// Construct with the socket path from config.
    pub fn new(socket_path: String) -> Self {
        Self { socket_path }
    }

    /// Start accepting connections; calls `handler` for each command received.
    pub async fn serve(
        &self,
        _handler: impl Fn(DaemonCommand) -> DaemonResponse + Send + 'static,
    ) -> Result<()> {
        todo!("bind Unix domain socket with interprocess and dispatch DaemonCommand messages")
    }
}
