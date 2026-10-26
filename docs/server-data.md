# What the server keeps

The database-dump audit for Quill's Phase 2 gate: every column of every
table, what it holds, and why holding it is acceptable. The gate is that a
full dump of the database holds no plaintext of what writers write and no
private keys.

Two tests keep this honest:

- **The plaintext-canary test** (`crates/quill-e2e/tests/gate.rs`) puts a
  random canary in everything a writer can write, then searches dumps of
  every table and the server's logs for it, and for every secret the
  clients hold.
- **The audit test** (`crates/quill-e2e/tests/audit.rs`) reads the schema
  after every migration and fails if a column isn't listed here, or is
  listed here but gone. A migration that adds a column must add it here,
  with its class and a reason.

## Classes

| Class        | Meaning                                                                                                   |
|--------------|-----------------------------------------------------------------------------------------------------------|
| `public`     | Published to anyone by design: the board, personas, tags, public keys and certificates.                   |
| `login`      | What a login needs to work: its name, a password hash, preferences, server-side session tokens.          |
| `moderation` | Plaintext about prompts and tags that reporters and administrators write, shown to administrators.        |
| `metadata`   | Ids, times, sizes, counts, log positions, and links between rows (who is in which session, and when).    |
| `ciphertext` | Encrypted on a client under keys the server never has: MLS messages and Welcomes, blobs, backups.        |
| `digest`     | A hash of a secret or a text the server doesn't keep.                                                     |
| `settings`   | The server's own configuration.                                                                           |

No column holds a writer's session text, a channel's name, a file, a file's
name or type, or any private key. Those exist only inside `ciphertext`.

## What the server can learn

From `metadata` and `public` columns, the server (and anyone with a dump):

- which personas a login has, and so which personas are the same person;
- which two personas are in each session, from which prompt, and since when;
- when each message was posted, its padded size (a multiple of 256 bytes),
  and whether it was a commit;
- each file's padded size (a power-of-two bucket) and when it was uploaded;
- how many devices each persona has, and how far each has read each session;
- when a login made a backup, and its size;
- that a request was sent, to which prompt, and whether it was accepted.

It can't learn what was written: pitches, notes, edits, deletions, replies,
channels, files, history sent between devices, backups, the recovery code,
or any private key. Federation (Phase 5) adds what the other server learns
about its own users' sessions, as the plan describes.

## Every column

### `_sqlx_migrations`

Not listed column by column: sqlx's own record of which migrations ran
(version, description, checksum, time). `settings` throughout.

### `account_notices`

| Column       | Class        | What                                                                                   |
|--------------|--------------|----------------------------------------------------------------------------------------|
| `id`         | `metadata`   | Notice id.                                                                             |
| `account_id` | `metadata`   | The login the notice is for.                                                           |
| `log_id`     | `metadata`   | The admin-log entry it came from.                                                      |
| `kind`       | `moderation` | What administrators did (a prompt or comment removed, a tag changed).                  |
| `fields`     | `moderation` | The tag, reason or announcement involved.                                              |
| `prompts`    | `public`     | The login's affected prompts as they were: id, title and handle, all public on the board. |
| `created_at` | `metadata`   | When.                                                                                  |
| `read_at`    | `metadata`   | When the login read it.                                                                |

### `accounts`

| Column          | Class      | What                                                                 |
|-----------------|------------|----------------------------------------------------------------------|
| `id`            | `metadata` | Login id.                                                            |
| `login_name`    | `login`    | The name the login signs in with. Never published.                   |
| `password_hash` | `login`    | Argon2id hash of the password.                                       |
| `is_admin`      | `login`    | Whether the login administers the server.                            |
| `created_at`    | `metadata` | When the login was made.                                             |
| `maturity_max`  | `login`    | The highest maturity the login chose to see on the board.            |
| `staff_name`    | `public`   | An administrator's chosen name for commenting as staff; shown on their staff comments, never the login name. |

### `actors`

| Column                 | Class      | What                                                                      |
|------------------------|------------|---------------------------------------------------------------------------|
| `id`                   | `metadata` | Persona id.                                                               |
| `account_id`           | `metadata` | The login that owns the persona. Never published; links its personas.    |
| `preferred_username`   | `public`   | The persona's handle.                                                     |
| `display_name`         | `public`   | Its display name.                                                         |
| `summary`              | `public`   | Its profile text.                                                         |
| `created_at`           | `metadata` | When it was made.                                                         |
| `identity_key`         | `public`   | The persona's identity public key. The private half never leaves clients. |
| `identity_key_version` | `public`   | Its version.                                                              |

### `admin_log`

| Column        | Class        | What                                                                        |
|---------------|--------------|-----------------------------------------------------------------------------|
| `id`          | `metadata`   | Entry id.                                                                   |
| `at`          | `metadata`   | When.                                                                       |
| `action`      | `moderation` | What happened (report received, prompt removed, tag changed).               |
| `report_id`   | `metadata`   | The report it concerns.                                                     |
| `prompt_id`   | `metadata`   | The prompt it concerns.                                                     |
| `prompt_hash` | `digest`     | SHA-256 of the prompt or comment as reported, so its text needn't be kept.  |
| `reason`      | `moderation` | The report's reason category.                                               |
| `admin_id`    | `metadata`   | The administrator who acted.                                                |
| `subject`     | `moderation` | The tag acted on, e.g. `genre:horror`.                                      |
| `detail`      | `moderation` | What changed about it.                                                      |
| `actor`       | `moderation` | Who acted: an administrator, the server, or a reporter (left unnamed).      |
| `admin_login` | `moderation` | The acting administrator's login name, as it was.                          |
| `comment_id`  | `metadata`   | The comment it concerns.                                                    |

### `admin_log_targets`

| Column       | Class        | What                                                   |
|--------------|--------------|--------------------------------------------------------|
| `log_id`     | `metadata`   | The entry.                                             |
| `account_id` | `metadata`   | A login the decision was against.                      |
| `login`      | `moderation` | Its login name, as it was, for administrators.         |

### `announcements`

| Column          | Class      | What                                                                   |
|-----------------|------------|------------------------------------------------------------------------|
| `id`            | `metadata` | Announcement id.                                                       |
| `title`         | `public`   | Its title, shown on the board.                                         |
| `body_md`       | `public`   | Its Markdown text.                                                     |
| `body_html`     | `public`   | The same, rendered and sanitized.                                      |
| `comments_open` | `settings` | Whether writers may comment on it.                                     |
| `sticky`        | `public`   | Whether its banner stays at the top of the board, not dismissible.     |
| `created_by`    | `metadata` | The administrator who posted it; readers see "the administrators".    |
| `created_at`    | `public`   | When it was posted.                                                    |
| `updated_at`    | `public`   | When it last changed.                                                  |
| `deleted_at`    | `metadata` | When an administrator deleted it; it then shows nowhere.               |

### `announcement_comments`

| Column            | Class        | What                                                                    |
|-------------------|--------------|-------------------------------------------------------------------------|
| `id`              | `metadata`   | Comment id.                                                             |
| `announcement_id` | `metadata`   | The announcement it's on.                                               |
| `actor_id`        | `public`     | The persona that wrote it, shown beside it (persona comments only).     |
| `voice`           | `public`     | Written as a persona, as "Administrators", or under a staff name.       |
| `account_id`      | `moderation` | The administrator behind an Administrators or staff comment; never shown to writers. |
| `staff_name`      | `public`     | The staff name it was posted under, as it was then.                     |
| `body_md`         | `public`     | Its Markdown text, public under the announcement.                       |
| `body_html`       | `public`     | The same, rendered and sanitized.                                       |
| `created_at`      | `public`     | When it was posted.                                                     |
| `removed_at`      | `moderation` | When an administrator removed it; then only its author sees it.         |
| `removed_reason`  | `moderation` | The reason category for the removal.                                    |

### `api_tokens`

| Column       | Class      | What                                                          |
|--------------|------------|---------------------------------------------------------------|
| `token_hash` | `digest`   | SHA-256 of a bearer token; the token itself is never stored.  |
| `account_id` | `metadata` | Its login.                                                    |
| `created_at` | `metadata` | When it was issued.                                           |
| `expires_at` | `metadata` | When it stops working.                                        |

### `backup_parts`

| Column        | Class        | What                                                                     |
|---------------|--------------|--------------------------------------------------------------------------|
| `account_id`  | `metadata`   | The login.                                                               |
| `generation`  | `metadata`   | Which upload the part belongs to.                                        |
| `part`        | `metadata`   | Its number.                                                              |
| `data`        | `ciphertext` | Part of an `age` file sealed with the recovery code, which the server never sees. |
| `uploaded_at` | `metadata`   | When.                                                                    |

### `backups`

| Column       | Class      | What                                 |
|--------------|------------|--------------------------------------|
| `account_id` | `metadata` | The login.                           |
| `generation` | `metadata` | The upload that is its backup.       |
| `parts`      | `metadata` | How many parts.                      |
| `bytes`      | `metadata` | Its size.                            |
| `created_at` | `metadata` | When it was committed.               |

### `blobs`

| Column        | Class        | What                                                                                     |
|---------------|--------------|------------------------------------------------------------------------------------------|
| `id`          | `metadata`   | Blob id (random).                                                                        |
| `group_id`    | `metadata`   | The session or device group it belongs to.                                               |
| `account_id`  | `metadata`   | The uploading login, for its quota.                                                      |
| `token_hash`  | `digest`     | SHA-256 of the capability token needed to fetch it; the token travels only inside MLS.  |
| `size`        | `metadata`   | Its padded size: a power-of-two bucket.                                                  |
| `data`        | `ciphertext` | An attachment or history part, AES-256-GCM under a key that travels only inside MLS.    |
| `seq`         | `metadata`   | The log position of the message that carries it.                                         |
| `uploaded_at` | `metadata`   | When.                                                                                    |

### `devices`

| Column        | Class      | What                                                                             |
|---------------|------------|----------------------------------------------------------------------------------|
| `id`          | `metadata` | Device id (random).                                                              |
| `actor_id`    | `metadata` | Its persona.                                                                     |
| `device_key`  | `public`   | The device's public key for that persona. The private half never leaves it.      |
| `key_version` | `public`   | The identity key version that certified it.                                      |
| `limited`     | `public`   | Whether it's a limited device.                                                   |
| `certificate` | `public`   | Its certificate, signed by the persona's identity key.                           |
| `revocation`  | `public`   | Its signed revocation, once revoked.                                             |

### `group_messages`

| Column      | Class        | What                                                                         |
|-------------|--------------|------------------------------------------------------------------------------|
| `group_id`  | `metadata`   | The session or device group.                                                 |
| `seq`       | `metadata`   | Its place in the log.                                                        |
| `epoch`     | `metadata`   | The MLS epoch it was sent in.                                                |
| `is_commit` | `metadata`   | Whether it changes the group (a commit) or carries content.                  |
| `data`      | `ciphertext` | An MLS `PrivateMessage`: the content and the sender are encrypted, padded.  |
| `logged_at` | `metadata`   | When it was posted.                                                          |

### `key_packages`

| Column        | Class      | What                                                                                     |
|---------------|------------|------------------------------------------------------------------------------------------|
| `id`          | `metadata` | Row id.                                                                                  |
| `device_id`   | `metadata` | The device it's for.                                                                     |
| `data`        | `public`   | An MLS KeyPackage: public keys and the device certificate, given to whoever starts a session. |
| `last_resort` | `public`   | Whether it's the device's last-resort package.                                           |
| `not_after`   | `public`   | When it expires.                                                                         |

### `link_mailbox`

| Column        | Class        | What                                                                                     |
|---------------|--------------|------------------------------------------------------------------------------------------|
| `link_id`     | `metadata`   | The link's id, shown in the link code. Lives 10 minutes; only its login can answer it.  |
| `account_id`  | `metadata`   | The login linking a device.                                                              |
| `key_package` | `public`     | The new device's KeyPackage (public keys); the code's fingerprint checks it.             |
| `welcome`     | `ciphertext` | The MLS Welcome to the device group, encrypted to that KeyPackage.                       |
| `from_seq`    | `metadata`   | Where in the device group's log the new device starts reading.                           |
| `expires_at`  | `metadata`   | When the mailbox goes.                                                                   |

### `mls_groups`

| Column           | Class      | What                                                              |
|------------------|------------|-------------------------------------------------------------------|
| `group_id`       | `metadata` | The MLS group id (random).                                        |
| `kind`           | `metadata` | A session, or a login's device group.                             |
| `account_id`     | `metadata` | For a device group, its login.                                    |
| `epoch`          | `metadata` | The epoch the next commit must be for.                            |
| `last_seq`       | `metadata` | The last log position used.                                       |
| `pruned_through` | `metadata` | How far the log has been deleted.                                 |

### `prompts`

| Column       | Class      | What                                                    |
|--------------|------------|---------------------------------------------------------|
| `id`         | `public`   | Prompt id.                                              |
| `actor_id`   | `public`   | The persona that posted it.                             |
| `title`      | `public`   | Its title, on the board.                                |
| `body_md`    | `public`   | Its text, on the board.                                 |
| `body_html`  | `public`   | The same, rendered.                                     |
| `maturity`   | `public`   | Its maturity rating.                                    |
| `state`      | `public`   | Open, busy or closed.                                   |
| `tag_ids`    | `public`   | Its tags.                                               |
| `search`     | `public`   | The full-text index of its title and text.              |
| `created_at` | `public`   | When it was posted.                                     |
| `updated_at` | `public`   | When it was last edited.                                |
| `removed_at` | `metadata` | When administrators removed it, if they did.            |

### `reports`

| Column             | Class        | What                                                                     |
|--------------------|--------------|--------------------------------------------------------------------------|
| `id`               | `metadata`   | Report id.                                                               |
| `prompt_id`        | `metadata`   | The prompt reported.                                                     |
| `comment_id`       | `metadata`   | The comment reported.                                                    |
| `kind`             | `moderation` | Whether a prompt or a comment was reported.                              |
| `reporter_account` | `metadata`   | Who reported it, for abuse control; never shown to the prompt's author. |
| `reason`           | `moderation` | The reason category.                                                     |
| `details`          | `moderation` | What the reporter wrote, for administrators.                             |
| `prompt_hash`      | `digest`     | SHA-256 of the prompt or comment as reported.                            |
| `created_at`       | `metadata`   | When.                                                                    |
| `resolved_at`      | `metadata`   | When it was dealt with.                                                  |
| `resolution`       | `moderation` | Discarded, or the prompt or comment removed.                             |
| `resolved_by`      | `metadata`   | The administrator who dealt with it.                                     |

### `reserved_handles`

| Column       | Class      | What                                                 |
|--------------|------------|------------------------------------------------------|
| `pattern`    | `settings` | A handle, or a word, no one may take.                |
| `kind`       | `settings` | Exact handle or word.                                |
| `source`     | `settings` | Built in, from the reserved-handles file, or added by administrators. |
| `created_at` | `settings` | When.                                                |

### `retired_handles`

| Column       | Class    | What                                                          |
|--------------|----------|---------------------------------------------------------------|
| `handle`     | `public` | A deleted persona's handle, never reused.                     |
| `retired_at` | `metadata` | When it was retired.                                        |

### `server_settings`

| Column       | Class      | What                                                  |
|--------------|------------|-------------------------------------------------------|
| `key`        | `settings` | A setting's name (new tags, attachments, comment visibility). |
| `value`      | `settings` | Its value.                                            |
| `updated_at` | `settings` | When it last changed.                                 |

### `session_acks`

| Column      | Class      | What                                                                       |
|-------------|------------|----------------------------------------------------------------------------|
| `group_id`  | `metadata` | The session.                                                               |
| `device_id` | `metadata` | A device of one of its personas.                                           |
| `seq`       | `metadata` | How far that device has read. Only the server sees it, never the other writer. |

### `session_members`

| Column     | Class      | What                                      |
|------------|------------|-------------------------------------------|
| `group_id` | `metadata` | The session.                              |
| `actor_id` | `metadata` | A persona taking part in it.              |

### `session_rejoins`

| Column       | Class      | What                                                                                     |
|--------------|------------|------------------------------------------------------------------------------------------|
| `id`         | `metadata` | Request id.                                                                              |
| `group_id`   | `metadata` | The session.                                                                             |
| `actor_id`   | `metadata` | The persona asking.                                                                      |
| `request`    | `public`   | The signed rejoin request: a KeyPackage, a certificate and device public keys to remove. |
| `created_at` | `metadata` | When.                                                                                    |

### `session_requests`

| Column                 | Class        | What                                                                          |
|------------------------|--------------|-------------------------------------------------------------------------------|
| `id`                   | `metadata`   | Request id.                                                                   |
| `prompt_id`            | `metadata`   | The prompt asked about.                                                       |
| `author_actor_id`      | `metadata`   | The prompt's author.                                                          |
| `requester_actor_id`   | `metadata`   | The persona asking.                                                           |
| `requester_account_id` | `metadata`   | Its login, for the limit of one waiting request per prompt.                   |
| `group_id`             | `metadata`   | The session's MLS group, made by the requester.                               |
| `epoch`                | `metadata`   | The group's epoch.                                                            |
| `state`                | `metadata`   | Pending, held, refused, declined, accepted or expired (the requester sees only waiting, session or no response). |
| `welcome`              | `ciphertext` | The MLS Welcome for the author's devices.                                     |
| `pitch`                | `ciphertext` | The pitch, an MLS `PrivateMessage` the server can't read.                     |
| `received_at`          | `metadata`   | When it arrived.                                                              |
| `expires_at`           | `metadata`   | When it expires unanswered.                                                   |

### `session_welcomes`

| Column      | Class        | What                                                          |
|-------------|--------------|---------------------------------------------------------------|
| `group_id`  | `metadata`   | The session.                                                  |
| `device_id` | `metadata`   | The device it's for.                                          |
| `welcome`   | `ciphertext` | An MLS Welcome, encrypted to that device's KeyPackage.        |
| `from_seq`  | `metadata`   | Where in the log the device starts reading.                   |

### `sessions`

| Column       | Class        | What                                                 |
|--------------|--------------|------------------------------------------------------|
| `group_id`   | `metadata`   | The session's group.                                 |
| `request_id` | `metadata`   | The request it began as.                             |
| `prompt_id`  | `metadata`   | Its prompt, while the prompt exists.                 |
| `welcome`    | `ciphertext` | The request's MLS Welcome, for devices in it.        |
| `ended_at`   | `metadata`   | When it ended.                                       |

### `tag_namespaces`

| Column       | Class    | What                                        |
|--------------|----------|---------------------------------------------|
| `name`       | `public` | A tag type, e.g. `genre`.                   |
| `display`    | `public` | Its display name.                           |
| `locked`     | `public` | Whether only administrators add tags to it. |
| `created_at` | `public` | When it was made.                           |
| `required`   | `public` | Whether every prompt needs one.             |
| `chip_bg`    | `public` | Its chip colours on the board.              |
| `chip_fg`    | `public` | The same.                                   |

### `tags`

| Column         | Class    | What                                              |
|----------------|----------|---------------------------------------------------|
| `id`           | `public` | Tag id.                                           |
| `namespace`    | `public` | Its type.                                         |
| `slug`         | `public` | Its normalized name.                              |
| `display`      | `public` | Its display name.                                 |
| `origin`       | `public` | Made by the server's administrators, or by a writer. |
| `instance`     | `public` | The server it comes from.                         |
| `canonical_id` | `public` | The tag it's an alias of.                         |
| `created_at`   | `public` | When it was made.                                 |
| `status`       | `public` | Visible, hidden, or pending approval.             |
| `chip_bg`      | `public` | Its chip colours.                                 |
| `chip_fg`      | `public` | The same.                                         |

### `web_sessions`

| Column       | Class      | What                                                                     |
|--------------|------------|--------------------------------------------------------------------------|
| `token_hash` | `digest`   | SHA-256 of the board's session cookie; the cookie itself isn't stored.   |
| `account_id` | `metadata` | Its login.                                                               |
| `csrf_token` | `login`    | The form token the board checks against, server-side by nature.          |
| `created_at` | `metadata` | When the board session began.                                            |
| `expires_at` | `metadata` | When it ends.                                                            |

## Uncertified devices are rejected

The gate's other half, proven by tests from earlier sub-phases:

| What                                                                                         | Test                                                                                     |
|----------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------|
| The server refuses certificates, revocations and identity keys that don't verify.           | `keys.rs`: `the_server_refuses_anything_that_wouldnt_verify`                             |
| Clients catch forged devices a dishonest server lets through.                               | `keys.rs`: `clients_catch_forgeries_the_server_lets_through`                             |
| The server refuses KeyPackages that don't belong to the device.                             | `key_packages.rs`: `the_server_refuses_packages_that_dont_belong_to_the_device`          |
| Claimers refuse KeyPackages a dishonest server substitutes.                                 | `key_packages.rs`: `claimers_refuse_packages_a_malicious_server_substitutes`             |
| Linking refuses a KeyPackage the server swapped.                                            | `linking.rs`: `the_code_catches_a_swapped_key_package_and_typed_words_work`              |
| Members refuse a commit adding the other writer's devices without a signed rejoin request.  | `sessions.rs`: `a_writer_cannot_add_the_other_writers_devices`                           |
| A session can't be given to another persona by the server.                                  | `sessions.rs`: `the_server_cannot_swap_the_other_writer`                                 |
| A rejoin request must be signed by the persona's identity key.                              | `backups.rs`: `a_rejoin_request_must_be_signed_by_the_persona`                           |
