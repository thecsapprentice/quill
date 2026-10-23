//! Deleting ciphertext (2g, 2k). A message goes once every device that
//! needs it has acknowledged it and the grace period has passed
//! ([`crate::sessions::prune`]), or once it's older than the retention
//! period, read or not ([`sweep`]). Blobs go with the messages that carry
//! them.
//!
//! Each group records how far its log has been deleted
//! (`mls_groups.pruned_through`). Reading from before that point gets
//! `410 Gone`: the reader missed commits and can't follow the group's keys,
//! so it must be let back in (sessions) or linked again (the device group).

use sqlx::{PgConnection, PgPool};

use crate::{AppState, error::ApiError};

/// How often the sweep runs.
pub const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// What a sweep deleted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Swept {
    /// Messages older than the retention period.
    pub aged: u64,
    /// Acknowledged messages past the grace period.
    pub acknowledged: u64,
    pub blobs: u64,
}

/// Deletes every group's messages logged before `now - days`, and records
/// how far each log is now deleted.
async fn delete_aged(conn: &mut PgConnection, days: u32) -> Result<u64, ApiError> {
    let deleted: i64 = sqlx::query_scalar(
        "WITH gone AS (
             DELETE FROM group_messages
             WHERE logged_at < now() - make_interval(days => $1)
             RETURNING group_id, seq
         ), top AS (
             SELECT group_id, max(seq) AS seq FROM gone GROUP BY group_id
         ), marked AS (
             UPDATE mls_groups g SET pruned_through = GREATEST(g.pruned_through, top.seq)
             FROM top WHERE g.group_id = top.group_id
             RETURNING 1
         )
         SELECT count(*) FROM gone",
    )
    .bind(i32::try_from(days).unwrap_or(i32::MAX))
    .fetch_one(&mut *conn)
    .await?;
    Ok(u64::try_from(deleted).unwrap_or(0))
}

/// Deletes blobs whose message is gone, and ones no message ever carried
/// after a day, in `group` or everywhere.
pub(crate) async fn delete_orphan_blobs(
    conn: &mut PgConnection,
    group: Option<&[u8]>,
) -> Result<u64, ApiError> {
    let deleted = sqlx::query(
        "DELETE FROM blobs b WHERE ($1::bytea IS NULL OR b.group_id = $1) AND (
             (b.seq IS NOT NULL AND NOT EXISTS (
                 SELECT 1 FROM group_messages m WHERE m.group_id = b.group_id AND m.seq = b.seq))
             OR (b.seq IS NULL AND b.uploaded_at < now() - interval '1 day'))",
    )
    .bind(group)
    .execute(&mut *conn)
    .await?;
    Ok(deleted.rows_affected())
}

/// One pass: deletes by age (unless `retention_days` is `None`, set by
/// `QUILL_RETENTION_DAYS=-1`), then what's acknowledged and past the grace
/// period (which no new acknowledgement may come along to trigger), then
/// orphaned blobs and stale rejoin requests.
pub async fn sweep(pool: &PgPool, retention_days: Option<u32>) -> Result<Swept, ApiError> {
    let mut tx = pool.begin().await?;
    let aged = match retention_days {
        Some(days) => delete_aged(&mut tx, days).await?,
        None => 0,
    };
    let groups: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT DISTINCT m.group_id FROM group_messages m
         JOIN sessions s ON s.group_id = m.group_id
         WHERE m.logged_at < now() - make_interval(days => $1)",
    )
    .bind(crate::sessions::GRACE_DAYS)
    .fetch_all(&mut *tx)
    .await?;
    let mut acknowledged = 0;
    for g in &groups {
        acknowledged += crate::sessions::prune_messages(&mut tx, g).await?;
    }
    let blobs = delete_orphan_blobs(&mut tx, None).await?;
    crate::sessions::drop_old_rejoins(&mut tx).await?;
    tx.commit().await?;
    Ok(Swept {
        aged,
        acknowledged,
        blobs,
    })
}

/// Runs [`sweep`] now and then every [`SWEEP_EVERY`], for the server's
/// lifetime.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(SWEEP_EVERY);
        loop {
            every.tick().await;
            match sweep(&state.pool, state.config.limits.retention_days).await {
                Ok(s) if s != Swept::default() => tracing::info!(
                    aged = s.aged,
                    acknowledged = s.acknowledged,
                    blobs = s.blobs,
                    "deleted old ciphertext"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "the retention sweep failed"),
            }
        }
    });
}
