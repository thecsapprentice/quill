//! Quill client core, shared by the CLI and (later) the desktop and web
//! clients.
//!
//! It provides the API client and the encrypted local store that holds
//! everything the client keeps. From Phase 2 it also owns all keys and MLS
//! state; that is why the server must never depend on it.

pub mod client;
pub mod devices;
pub mod keys;
pub mod mls;
pub mod profile;
pub mod store;
pub mod words;

pub use client::{Client, CoreError};
pub use profile::Profile;
pub use store::{Store, StoreError};
