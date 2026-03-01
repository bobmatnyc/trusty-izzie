//! trusty-daemon — the background sync process for trusty-izzie.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

use trusty_core::{init_logging, load_config};
use trusty_daemon::{ipc::IpcServer, DaemonLoop};

/// Command-line interface for the daemon process.
#[derive(Parser)]
#[command(name = "trusty-daemon", about = "trusty-izzie background sync daemon")]
struct Cli {
    #[command(subcommand)]
    command: DaemonCmd,

    /// Path to a custom config file.
    #[arg(long, global = true)]
    config: Option<String>,
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// Start the daemon (background by default).
    Start {
        /// Run in the foreground instead of daemonising.
        #[arg(long)]
        foreground: bool,
    },
    /// Send a stop signal to a running daemon.
    Stop,
    /// Print the daemon's current status.
    Status,
    /// Stop and restart the daemon.
    Restart,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config(cli.config.as_deref()).await?;

    init_logging(&std::env::var("TRUSTY_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()));

    match cli.command {
        DaemonCmd::Start { foreground } => {
            info!("starting trusty-daemon");
            if !foreground {
                // TODO: daemonise: write PID file, redirect stdout/stderr to log file
                todo!("daemonise process (fork, setsid, redirect stdio, write PID)")
            }
            run_daemon(config).await?;
        }
        DaemonCmd::Stop => {
            send_ipc_command(&config.daemon.ipc_socket, "shutdown").await?;
        }
        DaemonCmd::Status => {
            send_ipc_command(&config.daemon.ipc_socket, "status").await?;
        }
        DaemonCmd::Restart => {
            send_ipc_command(&config.daemon.ipc_socket, "shutdown").await?;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            run_daemon(config).await?;
        }
    }

    Ok(())
}

/// Run the daemon event loop: IPC server + email polling.
async fn run_daemon(config: trusty_models::config::AppConfig) -> Result<()> {
    let ipc_server = IpcServer::new(config.daemon.ipc_socket.clone());
    let daemon_loop = DaemonLoop::new(config.daemon.clone());

    // Set up Ctrl-C / SIGTERM shutdown
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx.send(());
    });

    // Run IPC server concurrently
    let ipc_task = tokio::spawn(async move {
        ipc_server
            .serve(|cmd| {
                use trusty_daemon::ipc::{DaemonCommand, DaemonResponse};
                match cmd {
                    DaemonCommand::Status => DaemonResponse::Status {
                        syncing: false,
                        last_sync: None,
                        last_message_count: 0,
                    },
                    _ => DaemonResponse::Ok,
                }
            })
            .await
    });

    daemon_loop
        .run(
            || async {
                // TODO: run full email sync cycle
                info!("sync tick (stub)");
                Ok(())
            },
            async {
                shutdown_rx.await.ok();
            },
        )
        .await?;

    ipc_task.abort();

    Ok(())
}

/// Send a control command to a running daemon via IPC.
async fn send_ipc_command(_socket_path: &str, _command: &str) -> Result<()> {
    todo!("connect to Unix socket and send DaemonCommand JSON, print DaemonResponse")
}
