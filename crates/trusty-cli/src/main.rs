//! trusty — the command-line interface for trusty-izzie.

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

use trusty_core::{init_logging, load_config};

/// A personal AI assistant that learns from your email.
#[derive(Parser)]
#[command(
    name = "trusty",
    about = "trusty-izzie personal AI assistant",
    version,
    propagate_version = true
)]
struct Cli {
    /// Path to a custom configuration file.
    #[arg(long, global = true)]
    config: Option<String>,

    /// Log level (trace, debug, info, warn, error).
    #[arg(long, global = true, env = "TRUSTY_LOG_LEVEL", default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start an interactive chat session.
    Chat(ChatArgs),
    /// Manage and search extracted entities.
    #[command(subcommand)]
    Entity(EntityCommand),
    /// Manage stored memories.
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Trigger an email sync.
    Sync(SyncArgs),
    /// Manage the background daemon.
    #[command(subcommand)]
    Daemon(DaemonCommand),
    /// Authenticate with Google.
    Auth(AuthArgs),
    /// Get or set configuration values.
    #[command(subcommand)]
    Config(ConfigCommand),
}

// --- Chat ---

#[derive(Args)]
struct ChatArgs {
    /// Continue an existing session by ID.
    #[arg(long)]
    session: Option<Uuid>,
}

// --- Entity ---

#[derive(Subcommand)]
enum EntityCommand {
    /// List all extracted entities.
    List(EntityListArgs),
    /// Search entities semantically.
    Search(EntitySearchArgs),
}

#[derive(Args)]
struct EntityListArgs {
    /// Filter by entity type (person, company, project, tool, topic, location, action_item).
    #[arg(long, short = 't')]
    r#type: Option<String>,
    /// Maximum number of results.
    #[arg(long, default_value = "50")]
    limit: usize,
}

#[derive(Args)]
struct EntitySearchArgs {
    /// The search query.
    query: String,
    /// Maximum number of results.
    #[arg(long, default_value = "10")]
    limit: usize,
}

// --- Memory ---

#[derive(Subcommand)]
enum MemoryCommand {
    /// List stored memories.
    List(MemoryListArgs),
    /// Search memories semantically.
    Search(MemorySearchArgs),
}

#[derive(Args)]
struct MemoryListArgs {
    /// Filter by category.
    #[arg(long, short = 'c')]
    category: Option<String>,
    /// Maximum number of results.
    #[arg(long, default_value = "20")]
    limit: usize,
}

#[derive(Args)]
struct MemorySearchArgs {
    /// The search query.
    query: String,
    /// Maximum number of results.
    #[arg(long, default_value = "10")]
    limit: usize,
}

// --- Sync ---

#[derive(Args)]
struct SyncArgs {
    /// Ignore the history cursor and re-scan recent mail.
    #[arg(long)]
    force: bool,
}

// --- Daemon ---

#[derive(Subcommand)]
enum DaemonCommand {
    /// Start the background daemon.
    Start {
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the running daemon.
    Stop,
    /// Show daemon status.
    Status,
}

// --- Auth ---

#[derive(Args)]
struct AuthArgs {
    /// The provider to authenticate with.
    #[arg(default_value = "google")]
    provider: String,
}

// --- Config ---

#[derive(Subcommand)]
enum ConfigCommand {
    /// Get the value of a configuration key.
    Get {
        /// The dotted config key (e.g. `openrouter.chat_model`).
        key: String,
    },
    /// Set a configuration value.
    Set {
        /// The dotted config key.
        key: String,
        /// The new value.
        value: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    init_logging(&cli.log_level);

    let config = load_config(cli.config.as_deref()).await?;

    match cli.command {
        Command::Chat(args) => run_chat(args, config).await,
        Command::Entity(cmd) => run_entity(cmd, config).await,
        Command::Memory(cmd) => run_memory(cmd, config).await,
        Command::Sync(args) => run_sync(args, config).await,
        Command::Daemon(cmd) => run_daemon(cmd, config).await,
        Command::Auth(args) => run_auth(args, config).await,
        Command::Config(cmd) => run_config(cmd, config).await,
    }
}

async fn run_chat(_args: ChatArgs, _config: trusty_models::config::AppConfig) -> Result<()> {
    todo!("start interactive REPL: read line → ChatEngine::chat → print reply")
}

async fn run_entity(cmd: EntityCommand, _config: trusty_models::config::AppConfig) -> Result<()> {
    match cmd {
        EntityCommand::List(args) => {
            println!("Listing entities (type={:?}, limit={})", args.r#type, args.limit);
            todo!("call API or GraphStore directly and pretty-print entity table")
        }
        EntityCommand::Search(args) => {
            println!("Searching entities: {}", args.query);
            todo!("call hybrid search and pretty-print results")
        }
    }
}

async fn run_memory(cmd: MemoryCommand, _config: trusty_models::config::AppConfig) -> Result<()> {
    match cmd {
        MemoryCommand::List(args) => {
            println!("Listing memories (category={:?}, limit={})", args.category, args.limit);
            todo!("query LanceDB and pretty-print memory list")
        }
        MemoryCommand::Search(args) => {
            println!("Searching memories: {}", args.query);
            todo!("call MemoryRecaller and pretty-print results")
        }
    }
}

async fn run_sync(args: SyncArgs, config: trusty_models::config::AppConfig) -> Result<()> {
    println!("Triggering sync (force={})", args.force);
    send_daemon_command(&config.daemon.ipc_socket, args.force).await
}

async fn run_daemon(cmd: DaemonCommand, config: trusty_models::config::AppConfig) -> Result<()> {
    match cmd {
        DaemonCommand::Start { foreground } => {
            println!("Starting daemon (foreground={})", foreground);
            todo!("invoke trusty-daemon binary or run inline")
        }
        DaemonCommand::Stop => {
            println!("Stopping daemon");
            todo!("send DaemonCommand::Shutdown via IPC")
        }
        DaemonCommand::Status => {
            println!("Checking daemon status");
            send_daemon_status(&config.daemon.ipc_socket).await
        }
    }
}

async fn run_auth(_args: AuthArgs, _config: trusty_models::config::AppConfig) -> Result<()> {
    println!("Starting Google OAuth2 flow...");
    todo!("build auth URL, open browser, start local redirect server, exchange code, persist tokens")
}

async fn run_config(cmd: ConfigCommand, config: trusty_models::config::AppConfig) -> Result<()> {
    match cmd {
        ConfigCommand::Get { key } => {
            println!("Getting config key: {key}");
            todo!("look up key in AppConfig and print value")
        }
        ConfigCommand::Set { key, value } => {
            println!("Setting config: {key} = {value}");
            todo!("persist key/value via SqliteStore::set_config and note file-backed keys too")
        }
    }
}

async fn send_daemon_command(_socket_path: &str, _force: bool) -> Result<()> {
    todo!("connect to daemon IPC socket and send Sync command")
}

async fn send_daemon_status(_socket_path: &str) -> Result<()> {
    todo!("connect to daemon IPC socket and print Status response")
}
