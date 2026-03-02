//! LLM-based entity and relationship extraction from email via OpenRouter.
//!
//! Uses a fast, small model (Mistral Small by default) to keep extraction
//! cost low. Strict confidence thresholds and noise filters guard quality.

pub mod extractor;
pub mod prompt;
pub mod types;

pub use extractor::{is_noise_email, EntityExtractor};
pub use types::{ExtractionResult, UserContext};
