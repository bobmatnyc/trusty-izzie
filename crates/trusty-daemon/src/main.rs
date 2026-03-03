//! trusty-daemon — the background sync process for trusty-izzie.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

use trusty_core::{init_logging, load_config};
use trusty_daemon::{ipc::IpcServer, DaemonLoop, EventDispatcher};
use trusty_models::config::AppConfig;
use trusty_models::{EventPayload, EventType};
use trusty_store::{SqliteStore, Store};

/// Single-tenant instance ID (SHA256 of primary email, first 16 hex chars).
const INSTANCE_ID: &str = "42a923e9bd673e38";

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

fn expand_data_dir(config: &AppConfig) -> PathBuf {
    let raw = &config.storage.data_dir;
    if raw.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(raw.replacen('~', &home, 1))
    } else {
        PathBuf::from(raw)
    }
}

/// Seed a recurring event if no pending event of that type exists.
fn seed_if_absent(
    sqlite: &SqliteStore,
    event_type: EventType,
    payload: EventPayload,
    scheduled_at: i64,
) -> Result<()> {
    let events = sqlite.list_events(Some("pending"), 100)?;
    if events.iter().any(|e| e.event_type == event_type) {
        return Ok(());
    }
    sqlite.enqueue_event(
        &event_type,
        &payload,
        scheduled_at,
        event_type.default_priority(),
        event_type.default_max_retries(),
        "system",
        None,
    )?;
    info!("Seeded {:?} event", event_type);
    Ok(())
}

/// Unix timestamp of next midnight UTC.
fn midnight_ts() -> i64 {
    use chrono::{Duration, TimeZone, Utc};
    let now = Utc::now();
    let tomorrow = (now + Duration::days(1))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    Utc.from_utc_datetime(&tomorrow).timestamp()
}

/// Run the daemon event loop: IPC server + event dispatcher.
async fn run_daemon(config: AppConfig) -> Result<()> {
    let data_dir = expand_data_dir(&config);
    let store = Arc::new(Store::open(&data_dir, INSTANCE_ID).await?);

    // Seed recurring events (idempotent — no-op if already pending).
    let now = chrono::Utc::now().timestamp();
    {
        let sqlite = &store.sqlite;
        seed_if_absent(
            sqlite,
            EventType::EmailSync,
            EventPayload::EmailSync { force: false },
            now,
        )?;
        seed_if_absent(
            sqlite,
            EventType::MemoryDecay,
            EventPayload::MemoryDecay { min_age_days: None },
            midnight_ts(),
        )?;
        seed_if_absent(
            sqlite,
            EventType::CalendarRefresh,
            EventPayload::CalendarRefresh { lookahead_days: 7 },
            now,
        )?;
        seed_if_absent(
            sqlite,
            EventType::ContactsSync,
            EventPayload::ContactsSync { force: false },
            now,
        )?;
    }

    let agents_dir = std::path::PathBuf::from(&config.agents.agents_dir);
    let openrouter_api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let dispatcher = EventDispatcher::new_with_agents(
        store,
        agents_dir,
        config.openrouter.base_url.clone(),
        openrouter_api_key,
    );
    let ipc_server = IpcServer::new(config.daemon.ipc_socket.clone());
    let daemon_loop = DaemonLoop::new();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx.send(());
    });

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
        .run(&dispatcher, async {
            shutdown_rx.await.ok();
        })
        .await?;

    ipc_task.abort();

    Ok(())
}

/// Send a control command to a running daemon via IPC.
async fn send_ipc_command(_socket_path: &str, _command: &str) -> Result<()> {
    todo!("connect to Unix socket and send DaemonCommand JSON, print DaemonResponse")
}
