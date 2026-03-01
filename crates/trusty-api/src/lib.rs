//! trusty-api — axum REST server exposing chat, entities, and memories.

pub mod handlers;
pub mod routes;
pub mod state;

pub use state::AppState;
