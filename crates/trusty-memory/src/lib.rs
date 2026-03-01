//! Memory recall, storage, and retrieval for the chat layer.
//!
//! This crate wraps `trusty-store` and `trusty-embeddings` to provide a
//! high-level interface for saving and retrieving memories during chat.

pub mod recall;
pub mod store;

pub use recall::MemoryRecaller;
pub use store::MemoryStore;
