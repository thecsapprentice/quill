//! Shared types for the Quill client API (`/api/v1`) and, later, the
//! ActivityPub `wr:` vocabulary.
//!
//! This crate is linked by both the server and clients, so it must never
//! depend on any cryptographic or MLS crate. Validation rules live here so the
//! server and every client enforce identical limits.

pub mod announcements;
pub mod api;
pub mod board;
pub mod color;
pub mod events;
pub mod groups;
pub mod handles;
pub mod keys;
pub mod limits;
pub mod notices;
pub mod requests;
pub mod sessions;
pub mod validate;

#[cfg(test)]
mod vectors;

/// Protocol versions this build speaks, advertised in `GET /api/v1/instance`.
pub const PROTOCOL_VERSIONS: &[&str] = &["0.1"];

/// Prefix of every client API route.
pub const API_PREFIX: &str = "/api/v1";
