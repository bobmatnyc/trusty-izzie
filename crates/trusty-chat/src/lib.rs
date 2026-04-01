//! Conversation engine for trusty-izzie.
//!
//! Assembles RAG context, dispatches tool calls, manages sessions,
//! and compresses long conversations.

pub mod context;
pub mod engine;
pub mod session;
pub mod skills;
pub mod tools;

pub use context::ContextAssembler;
pub use engine::{ChatEngine, ProgressCallback};
pub use session::SessionManager;
