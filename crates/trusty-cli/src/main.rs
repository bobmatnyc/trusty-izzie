//! trusty — the command-line interface for trusty-izzie.

pub mod log;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

use trusty_chat::context::ContextAssembler;
use trusty_chat::engine::ChatEngine;
use trusty_chat::session::SessionManager;
use trusty_core::{init_logging, load_config};
use trusty_email::auth::{generate_pkce_pair, GoogleAuthClient};
use trusty_models::config::AppConfig;
use trusty_models::entity::EntityType;
use trusty_models::memory::MemoryCategory;
use trusty_store::{SqliteStore, Store};

use crate::log::{append_interaction, InteractionLog};

const INSTANCE_ID: &str = "42a923e9bd673e38";
const SESSION_KEY: &str = "chat:current_session";

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
    #[arg(long, global = true, env = "TRUSTY_LOG_LEVEL", default_value = "warn")]
    log_level: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send a message and continue the current session.
    Chat(ChatArgs),
    /// Forget the current session so the next chat starts fresh.
    Clear,
    /// Manage chat sessions.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Manage and search extracted entities.
    #[command(subcommand)]
    Entity(EntityCommand),
    /// Manage stored memories.
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Trigger an email sync.
    Sync(SyncArgs),
    /// Authenticate with Google.
    Auth,
    /// Get or set configuration values.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Show process status for daemon and API server.
    Status,
    /// Show version and build information.
    Version,
}

// ── Chat ─────────────────────────────────────────────────────────────────────

#[derive(Args)]
struct ChatArgs {
    /// Message to send (reads stdin if omitted).
    message: Option<String>,
    /// Start a fresh session instead of continuing.
    #[arg(long)]
    new: bool,
    /// Use a specific session UUID.
    #[arg(long)]
    session: Option<Uuid>,
    /// Dry-run: call the LLM but do not save memories or persist the session.
    /// Shows what would have been written.
    #[arg(long = "test")]
    dry_run: bool,
}

// ── Session ───────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
enum SessionCommand {
    /// Show the 10 most recent sessions.
    List,
    /// Show all messages in a session.
    Show {
        /// Session UUID.
        uuid: Uuid,
    },
}

// ── Entity ────────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
enum EntityCommand {
    /// List extracted entities.
    List(EntityListArgs),
    /// Search entities by text.
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

// ── Memory ────────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
enum MemoryCommand {
    /// List stored memories.
    List(MemoryListArgs),
    /// Search memories by text.
    Search(MemorySearchArgs),
}

#[derive(Args)]
struct MemoryListArgs {
    /// Filter by category (e.g. person_fact, user_preference).
    #[arg(long, short = 'c')]
    category: Option<String>,
    /// Maximum number of results.
    #[arg(long, default_value = "20")]
    limit: usize,
}

#[derive(Args)]
struct MemorySearchArgs {
    /// The search query (substring match).
    query: String,
    /// Maximum number of results.
    #[arg(long, default_value = "10")]
    limit: usize,
}

// ── Sync ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
struct SyncArgs {
    /// Ignore the history cursor and re-scan recent mail.
    #[arg(long)]
    force: bool,
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
enum ConfigCommand {
    /// Get the value of a configuration key.
    Get {
        /// The dotted config key (e.g. `openrouter.chat_model`).
        key: String,
    },
    /// Set a configuration value (stored in SQLite KV).
    Set {
        /// The dotted config key.
        key: String,
        /// The new value.
        value: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    init_logging(&cli.log_level);
    let config = load_config(cli.config.as_deref()).await?;

    match cli.command {
        Command::Chat(args) => run_chat(args, config).await,
        Command::Clear => run_clear(config).await,
        Command::Session(cmd) => run_session(cmd, config).await,
        Command::Entity(cmd) => run_entity(cmd, config).await,
        Command::Memory(cmd) => run_memory(cmd, config).await,
        Command::Sync(args) => run_sync(args),
        Command::Auth => run_auth(config).await,
        Command::Config(cmd) => run_config(cmd, config).await,
        Command::Status => run_status(),
        Command::Version => run_version(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn data_dir(config: &AppConfig) -> PathBuf {
    let raw = &config.storage.data_dir;
    if raw.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(raw.replacen('~', &home, 1))
    } else {
        PathBuf::from(raw)
    }
}

/// Open SqliteStore at `<data_dir>/trusty.db`.
fn open_sqlite(config: &AppConfig) -> Result<Arc<SqliteStore>> {
    let db_path = data_dir(config).join(&config.storage.sqlite_path);
    let store = SqliteStore::open(&db_path)?;
    Ok(Arc::new(store))
}

/// Open full Store (LanceDB + Kuzu + SQLite).
async fn open_store(config: &AppConfig) -> Result<Arc<Store>> {
    let dir = data_dir(config);
    Ok(Arc::new(Store::open(&dir, INSTANCE_ID).await?))
}

fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

fn print_you(msg: &str) {
    if is_tty() {
        println!("\x1b[90myou \u{203a}\x1b[0m {msg}");
    } else {
        println!("you > {msg}");
    }
}

fn print_izzie(reply: &str) {
    if is_tty() {
        println!("\x1b[36mizzie \u{203a}\x1b[0m {reply}");
    } else {
        println!("izzie > {reply}");
    }
}

fn entity_type_label(t: &EntityType) -> &'static str {
    match t {
        EntityType::Person => "Person",
        EntityType::Company => "Company",
        EntityType::Project => "Project",
        EntityType::Tool => "Tool",
        EntityType::Topic => "Topic",
        EntityType::Location => "Location",
        EntityType::ActionItem => "ActionItem",
    }
}

fn memory_category_label(c: &MemoryCategory) -> &'static str {
    match c {
        MemoryCategory::UserPreference => "user_preference",
        MemoryCategory::PersonFact => "person_fact",
        MemoryCategory::ProjectFact => "project_fact",
        MemoryCategory::CompanyFact => "company_fact",
        MemoryCategory::RecurringEvent => "recurring_event",
        MemoryCategory::Decision => "decision",
        MemoryCategory::Event => "event",
        MemoryCategory::General => "general",
    }
}

/// Map a user-supplied type string (case-insensitive) to a Kuzu label.
fn entity_type_lance_label(s: &str) -> &'static str {
    match s.to_lowercase().as_str() {
        "person" => "Person",
        "company" => "Company",
        "project" => "Project",
        "tool" => "Tool",
        "topic" => "Topic",
        "location" => "Location",
        "action_item" | "actionitem" => "ActionItem",
        _ => "Unknown",
    }
}

#[allow(dead_code)]
fn entity_type_kuzu_label(s: &str) -> Option<&'static str> {
    match s.to_lowercase().as_str() {
        "person" => Some("Person"),
        "company" => Some("Company"),
        "project" => Some("Project"),
        "tool" => Some("Tool"),
        "topic" => Some("Topic"),
        "location" => Some("Location"),
        "action_item" | "actionitem" => Some("ActionItem"),
        _ => None,
    }
}

fn format_unix_ts(ts: i64) -> String {
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "unknown".to_string(),
    }
}

fn read_stdin_message() -> Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        bail!("no message provided (pass a message argument or pipe text via stdin)");
    }
    Ok(trimmed)
}

/// Truncate a string to at most `max` chars, appending "..." if cut.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

/// First `n` chars of `s` (unicode-safe via char boundary).
fn preview(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        // Walk back to a char boundary
        let mut end = n;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }
}

// ── Command implementations ───────────────────────────────────────────────────

async fn run_chat(args: ChatArgs, config: AppConfig) -> Result<()> {
    let t0 = Instant::now();
    let dry_run = args.dry_run;

    let message = match args.message {
        Some(m) => m,
        None => read_stdin_message()?,
    };

    let sqlite = open_sqlite(&config)?;
    let session_manager = SessionManager::new(Arc::clone(&sqlite));

    // Resolve which session to use
    let mut session = if let Some(explicit_id) = args.session {
        // --session <uuid>: load it, create fresh if not found
        match session_manager.load(explicit_id).await? {
            Some(s) => s,
            None => {
                eprintln!("Session {explicit_id} not found; starting fresh.");
                SessionManager::new_session(INSTANCE_ID)
            }
        }
    } else if args.new {
        // --new: always start fresh
        SessionManager::new_session(INSTANCE_ID)
    } else {
        // default: resume stored session
        let stored = sqlite.get_config(SESSION_KEY)?;
        match stored.and_then(|s| if s.is_empty() { None } else { Some(s) }) {
            Some(id_str) => match id_str.parse::<Uuid>() {
                Ok(uid) => match session_manager.load(uid).await? {
                    Some(s) => s,
                    None => SessionManager::new_session(INSTANCE_ID),
                },
                Err(_) => SessionManager::new_session(INSTANCE_ID),
            },
            None => SessionManager::new_session(INSTANCE_ID),
        }
    };

    let store = Store::open(&data_dir(&config), INSTANCE_ID).await?;
    let assembler = ContextAssembler::new(5, 10).with_lance(Arc::clone(&store.lance));

    let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let engine = ChatEngine::new_with_context(
        config.openrouter.base_url.clone(),
        api_key,
        config.openrouter.chat_model.clone(),
        config.chat.max_tool_iterations,
        assembler,
    );

    print_you(&message);
    let response = engine.chat(&mut session, &message).await?;
    print_izzie(&response.reply);

    let duration_ms = t0.elapsed().as_millis() as u64;
    let reply_preview = preview(&response.reply, 120);

    if dry_run {
        println!("\n[TEST MODE — read-only, nothing will be saved]\n");
        // memories_saved is 0 in dry_run (saving is skipped)
        println!("Would save 0 memories:");
    } else {
        session_manager.save(&session).await?;
        sqlite.set_config(SESSION_KEY, &session.id.to_string())?;
    }

    let log_path = data_dir(&config).join("interactions.jsonl");
    let entry = InteractionLog {
        ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        command: "chat",
        session_id: Some(session.id.to_string()),
        message: Some(&message),
        reply_preview: Some(reply_preview),
        tokens: None,
        duration_ms,
        memories_saved: Some(0),
        query: None,
        result_count: None,
        dry_run,
    };
    let _ = append_interaction(&log_path, &entry);

    Ok(())
}

async fn run_clear(config: AppConfig) -> Result<()> {
    let sqlite = open_sqlite(&config)?;
    let previous = sqlite.get_config(SESSION_KEY)?;
    sqlite.set_config(SESSION_KEY, "")?;
    println!("Session cleared. Next chat will start fresh.");
    if let Some(prev) = previous.filter(|s| !s.is_empty()) {
        println!("  (Cleared session: {prev})");
    }
    Ok(())
}

async fn run_session(cmd: SessionCommand, config: AppConfig) -> Result<()> {
    match cmd {
        SessionCommand::List => {
            let sqlite = open_sqlite(&config)?;
            let active_id = sqlite.get_config(SESSION_KEY)?.filter(|s| !s.is_empty());
            let sessions = sqlite.list_recent_sessions(10)?;
            if sessions.is_empty() {
                println!("No sessions found.");
                return Ok(());
            }
            println!("Recent sessions ({}):", sessions.len());
            for (i, (id, title, last_active)) in sessions.iter().enumerate() {
                let marker = if active_id.as_deref() == Some(id.as_str()) {
                    "* "
                } else {
                    "  "
                };
                let title_str = title.as_deref().unwrap_or("(no title)");
                println!(
                    "{marker}{:>2}. {}  {}  \"{}\"",
                    i + 1,
                    format_unix_ts(*last_active),
                    id,
                    title_str
                );
            }
        }
        SessionCommand::Show { uuid } => {
            let sqlite = open_sqlite(&config)?;
            let id_str = uuid.to_string();
            let session_row = sqlite.get_session(&id_str)?;
            match session_row {
                None => bail!("Session {uuid} not found."),
                Some(_) => {
                    let messages = sqlite.get_messages(&id_str)?;
                    println!("Session: {uuid}");
                    println!("{}", "\u{2500}".repeat(41));
                    for (_msg_id, role, content, _ts) in &messages {
                        if role == "user" {
                            print_you(content);
                        } else {
                            print_izzie(content);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn run_entity(cmd: EntityCommand, config: AppConfig) -> Result<()> {
    let t0 = Instant::now();
    let store = open_store(&config).await?;
    let log_path = data_dir(&config).join("interactions.jsonl");

    match cmd {
        EntityCommand::List(args) => {
            // Map user-facing type name to the canonical capitalized form stored in LanceDB.
            let lance_type = args.r#type.as_deref().map(entity_type_lance_label);
            if let Some(ref t) = args.r#type {
                if lance_type == Some("Unknown") {
                    bail!(
                        "unknown entity type '{}'. Valid: person, company, project, tool, topic, location, action_item",
                        t
                    );
                }
            }
            let entities = store.lance.list_entities(lance_type, args.limit).await?;
            if entities.is_empty() {
                println!("No entities found.");
            } else {
                print_entity_table(&entities);
            }
            let _ = append_interaction(
                &log_path,
                &InteractionLog {
                    ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    command: "entity_list",
                    session_id: None,
                    message: None,
                    reply_preview: None,
                    tokens: None,
                    duration_ms: t0.elapsed().as_millis() as u64,
                    memories_saved: None,
                    query: None,
                    result_count: Some(entities.len()),
                    dry_run: false,
                },
            );
        }
        EntityCommand::Search(args) => {
            let entities = store
                .lance
                .search_entities_text(&args.query, args.limit)
                .await?;
            if entities.is_empty() {
                println!("No entities matched \"{}\".", args.query);
            } else {
                println!("Search results for \"{}\":", args.query);
                print_entity_table(&entities);
            }
            let _ = append_interaction(
                &log_path,
                &InteractionLog {
                    ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    command: "entity_search",
                    session_id: None,
                    message: None,
                    reply_preview: None,
                    tokens: None,
                    duration_ms: t0.elapsed().as_millis() as u64,
                    memories_saved: None,
                    query: Some(&args.query),
                    result_count: Some(entities.len()),
                    dry_run: false,
                },
            );
        }
    }
    Ok(())
}

fn print_entity_table(entities: &[trusty_models::entity::Entity]) {
    let type_w = 10usize;
    let val_w = 22usize;
    let norm_w = 22usize;
    let conf_w = 10usize;
    println!(
        "{:<type_w$}  {:<val_w$}  {:<norm_w$}  {:<conf_w$}",
        "Type", "Value", "Normalized", "Confidence"
    );
    println!(
        "{}\u{2500}\u{2500}  {}\u{2500}\u{2500}  {}\u{2500}\u{2500}  {}\u{2500}\u{2500}",
        "\u{2500}".repeat(type_w - 2),
        "\u{2500}".repeat(val_w - 2),
        "\u{2500}".repeat(norm_w - 2),
        "\u{2500}".repeat(conf_w - 2),
    );
    for e in entities {
        println!(
            "{:<type_w$}  {:<val_w$}  {:<norm_w$}  {:.2}",
            entity_type_label(&e.entity_type),
            truncate(&e.value, val_w),
            truncate(&e.normalized, norm_w),
            e.confidence,
        );
    }
}

async fn run_memory(cmd: MemoryCommand, config: AppConfig) -> Result<()> {
    let t0 = Instant::now();
    let store = open_store(&config).await?;
    let log_path = data_dir(&config).join("interactions.jsonl");

    match cmd {
        MemoryCommand::List(args) => {
            let mut memories = store.lance.list_memories(args.limit * 4).await?;
            if let Some(cat) = &args.category {
                memories.retain(|m| memory_category_label(&m.category) == cat.as_str());
            }
            memories.truncate(args.limit);
            if memories.is_empty() {
                println!("No memories found.");
            } else {
                print_memory_table(&memories);
            }
            let _ = append_interaction(
                &log_path,
                &InteractionLog {
                    ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    command: "memory_list",
                    session_id: None,
                    message: None,
                    reply_preview: None,
                    tokens: None,
                    duration_ms: t0.elapsed().as_millis() as u64,
                    memories_saved: None,
                    query: None,
                    result_count: Some(memories.len()),
                    dry_run: false,
                },
            );
        }
        MemoryCommand::Search(args) => {
            let q = args.query.to_lowercase();
            let mut memories = store.lance.list_memories(500).await?;
            memories.retain(|m| m.content.to_lowercase().contains(&q));
            memories.truncate(args.limit);
            if memories.is_empty() {
                println!("No memories matched \"{}\".", args.query);
            } else {
                println!("Search results for \"{}\":", args.query);
                print_memory_table(&memories);
            }
            let _ = append_interaction(
                &log_path,
                &InteractionLog {
                    ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    command: "memory_search",
                    session_id: None,
                    message: None,
                    reply_preview: None,
                    tokens: None,
                    duration_ms: t0.elapsed().as_millis() as u64,
                    memories_saved: None,
                    query: Some(&args.query),
                    result_count: Some(memories.len()),
                    dry_run: false,
                },
            );
        }
    }
    Ok(())
}

fn print_memory_table(memories: &[trusty_models::memory::Memory]) {
    let cat_w = 16usize;
    let content_w = 42usize;
    let imp_w = 10usize;
    let str_w = 8usize;
    println!(
        "{:<cat_w$}  {:<content_w$}  {:<imp_w$}  {:<str_w$}",
        "Category", "Content", "Importance", "Strength"
    );
    println!(
        "{}\u{2500}\u{2500}  {}\u{2500}\u{2500}  {}\u{2500}\u{2500}  {}\u{2500}\u{2500}",
        "\u{2500}".repeat(cat_w - 2),
        "\u{2500}".repeat(content_w - 2),
        "\u{2500}".repeat(imp_w - 2),
        "\u{2500}".repeat(str_w - 2),
    );
    for m in memories {
        // strength is not exposed on Memory; use importance as displayed value
        println!(
            "{:<cat_w$}  {:<content_w$}  {:<imp_w$.2}  {:<str_w$.2}",
            memory_category_label(&m.category),
            truncate(&m.content, content_w),
            m.importance,
            m.importance, // strength not available in Memory struct; mirrors importance
        );
    }
}

fn run_sync(args: SyncArgs) -> Result<()> {
    println!(
        "Sync not yet wired to daemon (force={}). Start the daemon and it will sync automatically.",
        args.force
    );
    Ok(())
}

async fn run_auth(config: AppConfig) -> Result<()> {
    let sqlite = open_sqlite(&config)?;

    // Read credentials from environment
    let client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();

    if client_id.is_empty() || client_secret.is_empty() {
        bail!(
            "Missing GOOGLE_CLIENT_ID or GOOGLE_CLIENT_SECRET.\n               Set them in .env or config/default.toml."
        );
    }

    let redirect_uri = "https://izzie.ngrok.dev/api/auth/google/callback".to_string();
    let auth_client = GoogleAuthClient::new(client_id, client_secret, redirect_uri);

    // Generate PKCE pair and store verifier for the axum callback handler.
    let (verifier, challenge) = generate_pkce_pair();
    sqlite.set_config("oauth_pkce_verifier", &verifier)?;

    let auth_url = auth_client.authorization_url_pkce(&challenge);

    println!("\nOpening Google OAuth consent page…");
    println!("If the browser does not open, visit:\n\n  {}\n", auth_url);

    // Try to open the browser (macOS: `open`, Linux: `xdg-open`)
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(&auth_url).status();

    // Poll SQLite for google_access_token written by the axum callback handler.
    println!("Waiting for OAuth callback via https://izzie.ngrok.dev/api/auth/google/callback …");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let token = sqlite.get_config("google_access_token")?;
        if token.as_deref().map(|t| !t.is_empty()).unwrap_or(false) {
            println!("Authenticated!");
            println!("  Tokens stored in SQLite (run 'trusty sync' to pull email)");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            bail!(
                "Timed out after 120s waiting for OAuth callback.\n\
                 Make sure trusty-telegram is running and the ngrok tunnel is active.\n\
                 Visit the URL above manually if needed."
            );
        }
    }
}

async fn run_config(cmd: ConfigCommand, config: AppConfig) -> Result<()> {
    let sqlite = open_sqlite(&config)?;
    match cmd {
        ConfigCommand::Get { key } => {
            let val = sqlite.get_config(&key)?;
            match val {
                Some(v) => println!("{key} = {v}"),
                None => println!("{key} is not set"),
            }
        }
        ConfigCommand::Set { key, value } => {
            sqlite.set_config(&key, &value)?;
            println!("{key} = {value}");
        }
    }
    Ok(())
}

fn run_status() -> Result<()> {
    println!("trusty-izzie status");
    for (name, pid_file) in &[
        ("daemon", "/tmp/trusty-daemon.pid"),
        ("api", "/tmp/trusty-api.pid"),
        ("telegram", "/tmp/trusty-telegram.pid"),
    ] {
        match std::fs::read_to_string(pid_file) {
            Ok(contents) => {
                let pid = contents.trim();
                if pid.is_empty() {
                    println!("  {name:<10} \u{25cb} stopped");
                } else {
                    // Verify the PID is still alive
                    let alive = is_pid_alive(pid);
                    if alive {
                        println!("  {name:<10} \u{25cf} running  (PID {pid})");
                    } else {
                        println!("  {name:<10} \u{25cb} stopped  (stale PID {pid})");
                    }
                }
            }
            Err(_) => {
                println!("  {name:<10} \u{25cb} stopped");
            }
        }
    }
    Ok(())
}

fn run_version() -> Result<()> {
    println!("trusty-izzie {}", env!("CARGO_PKG_VERSION"));
    println!("  git:   {}", env!("TRUSTY_GIT_HASH"));
    println!("  built: {}", env!("TRUSTY_BUILD_DATE"));
    Ok(())
}

/// Check whether a process with the given PID string is alive.
/// On Unix: reads /proc/<pid> (Linux) or checks /proc via existence; falls back
/// to checking if /proc/<pid> exists. On macOS /proc is absent, so we use
/// `kill -0` via the standard library's process-spawning.
fn is_pid_alive(pid_str: &str) -> bool {
    let Ok(pid) = pid_str.trim().parse::<u32>() else {
        return false;
    };
    // Check /proc/<pid> on Linux; on macOS fall back to kill -0 via sh.
    let proc_path = format!("/proc/{pid}");
    if std::path::Path::new(&proc_path).exists() {
        return true;
    }
    // macOS / BSD: use `kill -0 <pid>` via a subprocess.
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
