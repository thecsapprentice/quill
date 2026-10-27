//! Automated moderation (#43): admin-written Rhai scripts, run in chains at
//! hooks in the board's write paths. Design and reasoning are in the
//! architecture document, "Automated moderation".

pub mod context;
pub mod data;
pub mod engine;
pub mod text;

pub use quill_proto::automod::{Hook, OnError};

/// What scripts and common data may use. Set from `QUILL_AUTOMOD_*`
/// environment variables; the defaults are safe for a small server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Rhai operations one script may run per post. Deterministic, so a
    /// script is stopped at the same point however busy the server is.
    pub max_operations: u64,
    /// Wall-clock cap for one script, a backstop to the operation count.
    pub time_ms: u64,
    /// Largest common data entry, in bytes. Scripts' string limit is set to
    /// at least this, so a full entry can always be read.
    pub data_entry_bytes: usize,
    /// All common data together, in bytes.
    pub data_total_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 1_000_000,
            time_ms: 100,
            data_entry_bytes: 1 << 20,
            data_total_bytes: 16 << 20,
        }
    }
}
