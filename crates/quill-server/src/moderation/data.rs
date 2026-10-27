//! Common data: named strings administrators keep for scripts, such as a
//! list of blocked words, so the same list isn't copied into every script.
//!
//! Every save is kept for history and undo. Scripts read a snapshot taken
//! when their chain starts, parsed once per version and shared rather than
//! copied (architecture document, "Automated moderation: common data").

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use quill_proto::automod::{
    ModerationData, ModerationDataSummary, ModerationDataVersion, SaveModerationData,
    valid_data_name,
};
use quill_proto::board::AdminAction;
use rhai::ImmutableString;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::text::{TextError, WordList};
use crate::{AppState, error::ApiError, reports::log_action};

/// One version of an entry, as scripts see it.
#[derive(Debug)]
pub struct Entry {
    pub name: String,
    pub version: i32,
    pub content: ImmutableString,
    lines: OnceLock<Arc<[ImmutableString]>>,
    words: OnceLock<Result<Arc<WordList>, TextError>>,
}

impl Entry {
    fn new(name: String, version: i32, content: String) -> Self {
        Self {
            name,
            version,
            content: content.into(),
            lines: OnceLock::new(),
            words: OnceLock::new(),
        }
    }

    /// The content as a list, parsed on first use; see [`parse_lines`].
    pub fn lines(&self) -> Arc<[ImmutableString]> {
        self.lines
            .get_or_init(|| {
                parse_lines(&self.content)
                    .into_iter()
                    .map(Into::into)
                    .collect()
            })
            .clone()
    }

    /// The lines built into a word list, on first use.
    pub fn word_list(&self) -> Result<Arc<WordList>, TextError> {
        self.words
            .get_or_init(|| WordList::new(&self.lines()).map(Arc::new))
            .clone()
    }
}

/// One entry per line, trimmed. Blank lines, and lines starting with `#`,
/// are left out.
pub fn parse_lines(content: &str) -> Vec<&str> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// The latest version of each entry scripts have read, so a version is
/// parsed once however many chains and posts read it. Snapshots keep their
/// own references, so replacing an entry here never changes a running chain.
#[derive(Debug, Default)]
pub struct DataCache(Mutex<HashMap<String, Arc<Entry>>>);

/// The common data a chain reads, fixed when the chain starts. Cheap to
/// clone: each script in the chain gets the same one.
#[derive(Debug, Default, Clone)]
pub struct Snapshot(Arc<HashMap<String, Arc<Entry>>>);

impl Snapshot {
    pub fn get(&self, name: &str) -> Option<&Arc<Entry>> {
        self.0.get(name)
    }

    /// A snapshot of entries made in memory, for dry runs and tests.
    pub fn from_entries(entries: impl IntoIterator<Item = (String, i32, String)>) -> Self {
        Self(Arc::new(
            entries
                .into_iter()
                .map(|(name, version, content)| {
                    (name.clone(), Arc::new(Entry::new(name, version, content)))
                })
                .collect(),
        ))
    }
}

/// Takes a snapshot of the named entries at their current versions. Names
/// with no entry are left out; upload checks keep that from happening.
pub async fn snapshot(
    pool: &PgPool,
    cache: &DataCache,
    names: &[String],
) -> Result<Snapshot, ApiError> {
    if names.is_empty() {
        return Ok(Snapshot::default());
    }
    let current: Vec<(String, i32)> =
        sqlx::query_as("SELECT name, version FROM moderation_data WHERE name = ANY($1)")
            .bind(names)
            .fetch_all(pool)
            .await?;
    let mut out = HashMap::new();
    let mut missing: Vec<(String, i32)> = Vec::new();
    {
        let cached = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
        for (name, version) in current {
            match cached.get(&name) {
                Some(e) if e.version == version => {
                    out.insert(name, e.clone());
                }
                _ => missing.push((name, version)),
            }
        }
    }
    if !missing.is_empty() {
        // Read from the history by exact version, so a save landing between
        // the two queries can't mix versions.
        let (names, versions): (Vec<String>, Vec<i32>) = missing.into_iter().unzip();
        let rows: Vec<(String, i32, String)> = sqlx::query_as(
            "SELECT v.name, v.version, v.content
             FROM moderation_data_versions v
             JOIN unnest($1::text[], $2::int[]) AS w(name, version)
               ON v.name = w.name AND v.version = w.version",
        )
        .bind(&names)
        .bind(&versions)
        .fetch_all(pool)
        .await?;
        let mut cached = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
        for (name, version, content) in rows {
            let entry = Arc::new(Entry::new(name.clone(), version, content));
            let newer = cached.get(&name).is_some_and(|e| e.version > version);
            if !newer {
                cached.insert(name.clone(), entry.clone());
            }
            out.insert(name, entry);
        }
    }
    Ok(Snapshot(Arc::new(out)))
}

/// [`snapshot`] with the server's own cache, for tests outside the crate.
#[doc(hidden)]
pub async fn snapshot_for_tests(state: &AppState, names: &[String]) -> Result<Snapshot, ApiError> {
    snapshot(&state.pool, &state.automod_data, names).await
}

fn check_name(name: &str) -> Result<(), ApiError> {
    if valid_data_name(name) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "{name:?} can't name common data: use lowercase letters, digits and underscores, up to 64 characters"
        )))
    }
}

#[derive(sqlx::FromRow)]
struct DataRow {
    name: String,
    description: String,
    content: String,
    version: i32,
    updated_at: OffsetDateTime,
    updated_by: Option<String>,
    used_by: Vec<String>,
}

pub async fn list(pool: &PgPool) -> Result<Vec<ModerationDataSummary>, ApiError> {
    let rows: Vec<(String, String, i32, i32, OffsetDateTime, Vec<String>)> = sqlx::query_as(
        "SELECT d.name, d.description, d.version, octet_length(d.content), d.updated_at,
                ARRAY(SELECT s.name FROM moderation_script_data u
                      JOIN moderation_scripts s ON s.id = u.script_id
                      WHERE u.data_name = d.name ORDER BY lower(s.name)) AS used_by
         FROM moderation_data d ORDER BY d.name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(name, description, version, size, updated_at, used_by)| ModerationDataSummary {
                name,
                description,
                version,
                size: u64::try_from(size).unwrap_or(0),
                updated_at,
                used_by,
            },
        )
        .collect())
}

pub async fn get(pool: &PgPool, name: &str) -> Result<ModerationData, ApiError> {
    let row: Option<DataRow> = sqlx::query_as(
        "SELECT d.name, d.description, d.content, d.version, d.updated_at,
                a.login_name AS updated_by,
                ARRAY(SELECT s.name FROM moderation_script_data u
                      JOIN moderation_scripts s ON s.id = u.script_id
                      WHERE u.data_name = d.name ORDER BY lower(s.name)) AS used_by
         FROM moderation_data d LEFT JOIN accounts a ON a.id = d.updated_by
         WHERE d.name = $1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or(ApiError::NotFound)?;
    Ok(ModerationData {
        name: row.name,
        description: row.description,
        content: row.content,
        version: row.version,
        updated_at: row.updated_at,
        updated_by: row.updated_by,
        used_by: row.used_by,
    })
}

/// Creates the entry or saves a new version, and logs it. Returns the new
/// version. Refused if it would break the size caps, or if `base_version`
/// is given and someone has saved since.
pub async fn save(
    state: &AppState,
    admin: Uuid,
    name: &str,
    body: &SaveModerationData,
) -> Result<i32, ApiError> {
    save_inner(state, admin, name, body, None).await
}

async fn save_inner(
    state: &AppState,
    admin: Uuid,
    name: &str,
    body: &SaveModerationData,
    restored_from: Option<i32>,
) -> Result<i32, ApiError> {
    check_name(name)?;
    let limits = &state.config.automod;
    if body.content.len() > limits.data_entry_bytes {
        return Err(ApiError::BadRequest(format!(
            "{name} is {} bytes; common data entries can be up to {} bytes",
            body.content.len(),
            limits.data_entry_bytes
        )));
    }
    let description = body.description.trim();
    if description.chars().count() > 500 {
        return Err(ApiError::BadRequest(
            "descriptions can be up to 500 characters".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    // Saves are rare; one at a time keeps the total-size check honest.
    sqlx::query("LOCK TABLE moderation_data IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let current: Option<i32> =
        sqlx::query_scalar("SELECT version FROM moderation_data WHERE name = $1")
            .bind(name)
            .fetch_optional(&mut *tx)
            .await?;
    if let (Some(base), Some(now)) = (body.base_version, current)
        && base != now
    {
        return Err(ApiError::Conflict(format!(
            "{name} was saved again (version {now}) after you started editing version {base}; reload it and make your change again"
        )));
    }
    let others: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(octet_length(content)), 0)::bigint FROM moderation_data WHERE name <> $1",
    )
    .bind(name)
    .fetch_one(&mut *tx)
    .await?;
    let total = usize::try_from(others)
        .unwrap_or(usize::MAX)
        .saturating_add(body.content.len());
    if total > limits.data_total_bytes {
        return Err(ApiError::BadRequest(format!(
            "all common data together can be up to {} bytes; this save would make it {total}",
            limits.data_total_bytes
        )));
    }

    let version = current.map_or(1, |v| v + 1);
    sqlx::query(
        "INSERT INTO moderation_data (name, description, content, version, updated_by, updated_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (name) DO UPDATE SET description = $2, content = $3, version = $4,
             updated_by = $5, updated_at = now()",
    )
    .bind(name)
    .bind(description)
    .bind(&body.content)
    .bind(version)
    .bind(admin)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO moderation_data_versions (name, version, content, created_by)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(name)
    .bind(version)
    .bind(&body.content)
    .bind(admin)
    .execute(&mut *tx)
    .await?;
    let detail = match restored_from {
        Some(old) => format!("version {version}, restoring version {old}"),
        None => format!("version {version}"),
    };
    log_action(
        &mut tx,
        admin,
        AdminAction::SaveModerationData,
        name,
        Some(&detail),
    )
    .await?;
    tx.commit().await?;
    Ok(version)
}

/// Deletes an entry and its history. Refused while any script reads it.
pub async fn delete(state: &AppState, admin: Uuid, name: &str) -> Result<(), ApiError> {
    let entry = get(&state.pool, name).await?;
    if !entry.used_by.is_empty() {
        return Err(in_use(name, &entry.used_by));
    }
    let mut tx = state.pool.begin().await?;
    let deleted = sqlx::query("DELETE FROM moderation_data WHERE name = $1")
        .bind(name)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            // A script that started using it since the check above.
            if e.as_database_error()
                .is_some_and(|d| d.is_foreign_key_violation())
            {
                in_use(name, &[])
            } else {
                e.into()
            }
        })?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    log_action(
        &mut tx,
        admin,
        AdminAction::DeleteModerationData,
        name,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn in_use(name: &str, scripts: &[String]) -> ApiError {
    if scripts.is_empty() {
        ApiError::Conflict(format!("{name} is used by a moderation script"))
    } else {
        ApiError::Conflict(format!(
            "{name} is used by these moderation scripts: {}. Change them first.",
            scripts.join(", ")
        ))
    }
}

/// Every saved version, newest first.
pub async fn history(pool: &PgPool, name: &str) -> Result<Vec<ModerationDataVersion>, ApiError> {
    let rows: Vec<(i32, i32, OffsetDateTime, Option<String>)> = sqlx::query_as(
        "SELECT v.version, octet_length(v.content), v.created_at, a.login_name
         FROM moderation_data_versions v LEFT JOIN accounts a ON a.id = v.created_by
         WHERE v.name = $1 ORDER BY v.version DESC",
    )
    .bind(name)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound);
    }
    Ok(rows
        .into_iter()
        .map(
            |(version, size, created_at, created_by)| ModerationDataVersion {
                version,
                size: u64::try_from(size).unwrap_or(0),
                created_at,
                created_by,
            },
        )
        .collect())
}

/// One saved version's content.
pub async fn version(pool: &PgPool, name: &str, version: i32) -> Result<String, ApiError> {
    sqlx::query_scalar(
        "SELECT content FROM moderation_data_versions WHERE name = $1 AND version = $2",
    )
    .bind(name)
    .bind(version)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Undo: saves an old version's content again, as a new version, so the
/// history keeps what was undone.
pub async fn restore(state: &AppState, admin: Uuid, name: &str, old: i32) -> Result<i32, ApiError> {
    let current = get(&state.pool, name).await?;
    let content = version(&state.pool, name, old).await?;
    let body = SaveModerationData {
        description: current.description,
        content,
        base_version: Some(current.version),
    };
    save_inner(state, admin, name, &body, Some(old)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_skip_blanks_and_comments() {
        let content = "# words to refuse\nspam\n\n  free crypto  \n#not this\nemoji # stays\n";
        assert_eq!(
            parse_lines(content),
            ["spam", "free crypto", "emoji # stays"]
        );
    }

    #[test]
    fn entries_parse_once() {
        let entry = Entry::new("blocked".into(), 1, "spam\nscam".into());
        let first = entry.lines();
        assert!(Arc::ptr_eq(&first, &entry.lines()));
        let words = entry.word_list().unwrap();
        assert!(Arc::ptr_eq(&words, &entry.word_list().unwrap()));
        assert_eq!(words.find_in("s c a m here"), ["scam"]);
    }
}
