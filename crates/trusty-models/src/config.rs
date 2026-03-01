//! Application configuration types, matching `config/default.toml`.

use serde::{Deserialize, Serialize};

/// Root application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// OpenRouter LLM API settings.
    pub openrouter: OpenRouterConfig,
    /// Storage backend paths.
    pub storage: StorageConfig,
    /// Background daemon settings.
    pub daemon: DaemonConfig,
    /// Entity extraction tuning knobs.
    pub extraction: ExtractionConfig,
    /// Chat engine settings.
    pub chat: ChatConfig,
    /// REST API server settings.
    pub api: ApiConfig,
}

/// OpenRouter API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterConfig {
    /// Base URL of the OpenRouter API.
    pub base_url: String,
    /// Model ID used for interactive chat completions.
    pub chat_model: String,
    /// Model ID used for entity/relationship extraction.
    pub extraction_model: String,
    /// Local ONNX model identifier for fastembed.
    pub embedding_model: String,
    /// Maximum tokens in a single completion response.
    pub max_tokens: u32,
}

/// Filesystem paths for each storage backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Root directory; tilde is expanded at runtime.
    pub data_dir: String,
    /// Sub-path for LanceDB vector store (relative to `data_dir`).
    pub lance_path: String,
    /// Sub-path for Kuzu graph database (relative to `data_dir`).
    pub kuzu_path: String,
    /// Sub-path for the SQLite database file (relative to `data_dir`).
    pub sqlite_path: String,
    /// Sub-path for the Tantivy full-text index (relative to `data_dir`).
    pub tantivy_path: String,
}

/// Background daemon runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Seconds between Gmail polling cycles.
    pub email_sync_interval_secs: u64,
    /// Whether the daemon should run email sync at all.
    pub enabled: bool,
    /// Path to the Unix domain socket used for IPC.
    pub ipc_socket: String,
}

/// Entity extraction tuning parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// Minimum LLM confidence to accept an entity or relationship.
    pub confidence_threshold: f32,
    /// Minimum times an entity must appear before it is persisted.
    pub min_occurrences: u32,
    /// Maximum relationships extracted per email to limit noise.
    pub max_relationships_per_email: usize,
}

/// Chat engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatConfig {
    /// Maximum number of tool call iterations per user turn.
    pub max_tool_iterations: u32,
    /// Number of memories to inject into the context window.
    pub context_memory_limit: usize,
    /// Number of entities to inject into the context window.
    pub context_entity_limit: usize,
    /// Compress session history once message count exceeds this threshold.
    pub session_compression_threshold: usize,
}

/// REST API server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// TCP port to bind the server.
    pub port: u16,
    /// IP address to bind the server.
    pub host: String,
}
