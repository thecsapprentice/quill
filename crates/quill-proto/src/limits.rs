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

/// Blobs a login may store at once, in bytes.
pub const DEFAULT_ACCOUNT_QUOTA_BYTES: u64 = 500 * 1024 * 1024;

/// The largest backup a login may keep, in bytes (2j).
pub const DEFAULT_MAX_BACKUP_BYTES: u64 = 256 * 1024 * 1024;

/// Backups are uploaded in parts of at most this many bytes.
pub const BACKUP_PART_BYTES: u64 = 8 * 1024 * 1024;

/// Group messages a login may post per minute.
pub const DEFAULT_MESSAGES_PER_MINUTE: u32 = 30;

/// The smallest blob size bucket.
pub const BLOB_MIN_BUCKET: u64 = 4 * 1024;

/// The bucket a blob of `len` bytes is padded to: the smallest power of two
/// from [`BLOB_MIN_BUCKET`] that holds it, or `max` itself when that's
/// smaller. `None` when it doesn't fit under `max`.
pub fn blob_bucket(len: u64, max: u64) -> Option<u64> {
    if len > max {
        return None;
    }
    let bucket = len.max(BLOB_MIN_BUCKET).checked_next_power_of_two()?;
    Some(bucket.min(max))
}

/// Whether `len` is a size a padded blob can have.
pub fn is_blob_bucket(len: u64, max: u64) -> bool {
    len == max || (len.is_power_of_two() && (BLOB_MIN_BUCKET..=max).contains(&len))
}

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

/// Announcements from administrators: the same bounds as prompts.
pub const ANNOUNCEMENT_TITLE_MAX_CHARS: usize = PROMPT_TITLE_MAX_CHARS;
pub const ANNOUNCEMENT_BODY_MAX_CHARS: usize = PROMPT_BODY_MAX_CHARS;
/// The server rules page: one document, so longer than a prompt.
pub const RULES_BODY_MAX_CHARS: usize = 50_000;
/// A comment on an announcement.
pub const COMMENT_BODY_MAX_CHARS: usize = 5000;
/// The name an administrator comments under as staff.
pub const STAFF_NAME_MAX_CHARS: usize = 40;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_sizes_round_up_to_buckets() {
        let max = DEFAULT_MAX_ATTACHMENT_BYTES;
        assert_eq!(blob_bucket(0, max), Some(4096));
        assert_eq!(blob_bucket(4096, max), Some(4096));
        assert_eq!(blob_bucket(4097, max), Some(8192));
        assert_eq!(blob_bucket(max, max), Some(max));
        assert_eq!(blob_bucket(max + 1, max), None);
        // A limit that isn't a power of two is its own last bucket.
        assert_eq!(blob_bucket(5000, 6000), Some(6000));
        for ok in [4096, 8192, max, 6000] {
            assert!(
                is_blob_bucket(ok, if ok == 6000 { 6000 } else { max }),
                "{ok}"
            );
        }
        for bad in [0, 1000, 4095, 5000, max * 2] {
            assert!(!is_blob_bucket(bad, max), "{bad}");
        }
    }
}
