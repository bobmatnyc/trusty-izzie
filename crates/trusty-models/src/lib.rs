//! Pure data models for trusty-izzie. No business logic, only data structures.
//!
//! This crate is the foundation of the dependency graph. Every other crate may
//! depend on it; it depends on nothing internal.

pub mod agent;
pub mod chat;
pub mod config;
pub mod email;
pub mod entity;
pub mod event;
pub mod memory;

pub use agent::{Account, AgentTask, OAuthToken, OpenLoopRow};
pub use chat::{ChatMessage, ChatSession, MessageRole, StructuredResponse, ToolCallRequest};
pub use config::{AppConfig, DaemonConfig, ExtractionConfig, OpenRouterConfig, StorageConfig};
pub use email::{EmailMessage, GmailHistoryCursor};
pub use entity::{Entity, EntityType, Relationship, RelationshipStatus, RelationshipType};
pub use event::{EventPayload, EventStatus, EventType, QueuedEvent};
pub use memory::{Memory, MemoryCategory};
