//! Server configuration, read from `QUILL_*` environment variables.

use std::net::SocketAddr;

use anyhow::{Context, Result};
use quill_proto::{api::Limits, board::TagPolicy};

/// The upstream repository. Operators running a modified server must point
/// `QUILL_SOURCE_URL` at their own source.
pub const DEFAULT_SOURCE_URL: &str = "https://github.com/thecsapprentice/quill";

#[derive(Debug, Clone)]
pub struct Config {
    /// Public host name of this instance, e.g. `writers.example`.
    pub domain: String,
    pub bind: SocketAddr,
    pub database_url: String,
    pub registration_open: bool,
    pub max_personas_per_account: i64,
    pub token_ttl_days: i64,
    /// Starting value of the "new tags" server setting. The live value is
    /// in the database; see [`crate::server_settings`].
    pub tag_policy: TagPolicy,
    /// Where users can download this server's source code. The AGPL requires
    /// offering it to everyone who uses the server over a network.
    pub source_url: String,
    /// A file of reserved handles synced at startup
    /// (`QUILL_RESERVED_HANDLES_FILE`): one per line, a word unless it starts
    /// with `=` (exact handle), `#` comments.
    pub reserved_handles_file: Option<std::path::PathBuf>,
    pub limits: Limits,
    /// Starting value of the "attachments" server setting
    /// (`QUILL_ATTACHMENTS`); the live value is in the database.
    pub attachments: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url =
            std::env::var("QUILL_DATABASE_URL").context("QUILL_DATABASE_URL must be set")?;
        Ok(Self {
            domain: var_or("QUILL_DOMAIN", "localhost"),
            bind: var_or("QUILL_BIND", "127.0.0.1:8080")
                .parse()
                .context("QUILL_BIND must be host:port")?,
            database_url,
            registration_open: parse_or("QUILL_REGISTRATION_OPEN", true)?,
            max_personas_per_account: parse_or("QUILL_MAX_PERSONAS", 20)?,
            token_ttl_days: parse_or("QUILL_TOKEN_TTL_DAYS", 30)?,
            tag_policy: tag_policy_from_env()?,
            source_url: var_or("QUILL_SOURCE_URL", DEFAULT_SOURCE_URL),
            reserved_handles_file: std::env::var_os("QUILL_RESERVED_HANDLES_FILE")
                .filter(|v| !v.is_empty())
                .map(Into::into),
            limits: Limits::default(),
            attachments: parse_or("QUILL_ATTACHMENTS", true)?,
        })
    }

    /// Defaults suitable for tests; the caller supplies the database.
    pub fn for_tests(database_url: impl Into<String>) -> Self {
        Self {
            domain: "localhost".into(),
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: database_url.into(),
            registration_open: true,
            max_personas_per_account: 20,
            token_ttl_days: 30,
            tag_policy: TagPolicy::Open,
            source_url: DEFAULT_SOURCE_URL.into(),
            reserved_handles_file: None,
            limits: Limits::default(),
            attachments: true,
        }
    }
}

/// `QUILL_TAG_POLICY` sets the starting value of the "new tags" setting;
/// administrators change it in Server settings afterwards. The old values
/// `user` and `both` mean `open`.
fn tag_policy_from_env() -> Result<TagPolicy> {
    match std::env::var("QUILL_TAG_POLICY").as_deref() {
        Err(_) | Ok("user" | "both") => Ok(TagPolicy::Open),
        Ok(v) => v
            .parse()
            .context("QUILL_TAG_POLICY must be open, approval or curated"),
    }
}

fn var_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn parse_or<T: std::str::FromStr>(name: &str, default: T) -> Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match std::env::var(name) {
        Ok(v) => v.parse().with_context(|| format!("{name} is invalid")),
        Err(_) => Ok(default),
    }
}
