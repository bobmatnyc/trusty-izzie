//! Gmail OAuth2 sync for trusty-izzie.
//!
//! Handles the full Gmail integration: OAuth2 token management, incremental
//! history polling via the Gmail History API, and message decoding.

pub mod auth;
pub mod client;
pub mod sync;

pub use auth::GoogleAuthClient;
pub use client::GmailClient;
pub use sync::EmailSyncer;
