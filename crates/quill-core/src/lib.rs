//! Quill client core, shared by the CLI and (later) the desktop and web
//! clients.
//!
//! Phase 0 provides the API client and a local profile. From Phase 2 this
//! crate also owns all MLS state; that is why the server must never depend on
//! it.

pub mod client;
pub mod profile;

pub use client::{Client, CoreError};
pub use profile::Profile;
