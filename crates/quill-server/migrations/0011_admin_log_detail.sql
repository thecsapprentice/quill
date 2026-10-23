-- Phase 1h: an admin-log entry's detail page lists every account it was
-- against with the prompts it touched, read from that entry's notices. One
-- decision can reach thousands of accounts, so look them up by entry.
CREATE INDEX account_notices_log_idx ON account_notices (log_id, account_id);
