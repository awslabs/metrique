//! JSON formatter integration.
//!
//! This module re-exports the pure JSON formatter types from
//! [`metrique-writer-format-json`](https://docs.rs/metrique-writer-format-json).

#[cfg(feature = "json")]
pub use metrique_writer_format_json::{Json, SampledJson};
