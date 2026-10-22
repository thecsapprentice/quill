//! Default size limits. Servers may lower or raise these in configuration and
//! publish the effective values in [`crate::api::InstanceInfo`].

/// Largest MLS object a client may post (also the largest padding bucket).
pub const DEFAULT_MAX_MESSAGE_BYTES: u64 = 64 * 1024;

/// Largest encrypted attachment blob.
pub const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

/// Largest JSON request body accepted by the client API outside blob uploads.
/// Covers base64 overhead on a maximum-size message plus envelope fields.
pub const DEFAULT_MAX_ENVELOPE_BYTES: u64 = DEFAULT_MAX_MESSAGE_BYTES * 4 / 3 + 8 * 1024;

/// Days before an unanswered session request expires.
pub const DEFAULT_REQUEST_EXPIRY_DAYS: u32 = 30;

/// Days ciphertext is kept for a device that has not acknowledged it.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// Login names and persona handles.
pub const USERNAME_MIN_CHARS: usize = 1;
pub const USERNAME_MAX_CHARS: usize = 30;

/// Passwords are bounded in bytes so a single login cannot make Argon2 hash
/// an arbitrarily large input.
pub const PASSWORD_MIN_CHARS: usize = 12;
pub const PASSWORD_MAX_BYTES: usize = 1024;

pub const DISPLAY_NAME_MAX_CHARS: usize = 100;
pub const SUMMARY_MAX_CHARS: usize = 2000;

/// Prompts.
pub const PROMPT_TITLE_MAX_CHARS: usize = 200;
pub const PROMPT_BODY_MAX_CHARS: usize = 20_000;
pub const PROMPT_MAX_TAGS: usize = 15;

/// Tags and tag namespaces.
pub const TAG_SLUG_MAX_CHARS: usize = 40;
pub const TAG_DISPLAY_MAX_CHARS: usize = 40;
pub const NAMESPACE_MAX_CHARS: usize = 32;

/// Free text attached to a report.
pub const REPORT_DETAILS_MAX_CHARS: usize = 2000;

/// Full-text search query length.
pub const SEARCH_QUERY_MAX_CHARS: usize = 200;

/// Page sizes for list endpoints.
pub const DEFAULT_PAGE_SIZE: u32 = 20;
pub const MAX_PAGE_SIZE: u32 = 100;
