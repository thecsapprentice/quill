# Roadmap: phases and sub-phases

Each phase from the [plan](plan.md#roadmap-and-testing-strategy) is split into
sub-phases. A sub-phase is one reviewable pull request: small enough to test
by hand, and finished only when its "done when" list holds and CI is green.
We work through them in order, one at a time. Sub-phases for later phases are
a first cut and get refined when the phase starts.

Status: **done** (merged or in review), **next**, **planned**.

## Phase 0: Foundations (done)

Workspace, CI with cargo-deny, audit and vet, Postgres schema, accounts and
personas, CLI skeleton, the server dependency ban.

## Phase 1: Prompt board (done)

| Sub-phase | Scope | Status |
| --- | --- | --- |
| 1a | Prompts, tags, search, reports, admin inbox: API and CLI | done (#1) |
| 1b | Server-rendered board (`quill-board-ui`), sessions, CSRF | done (#2) |
| 1c | Docker setup and demo data | done |
| 1d | Admin settings: tag management and required tag types | done |
| 1e | Tag moderation: new-tag switch, hidden tags, tag requests | done |
| 1f | Tag picker and tag colours | done |
| 1g | Persona handle rules | done |
| 1h | Account moderation notices | done |

### 1c: Docker setup and demo data

- `Dockerfile` and `compose.yaml` for local testing (done).
- A demo-data seeder, run by `docker compose --profile demo run --rm seed`:
  at least 4 accounts, 1 to 3 personas each, 100+ prompts spread across tags,
  maturity ratings, states and dates, plus a few reports and one admin.
- The seeder (`quill-seed`, in `quill-devtools`) is a separate development
  binary with its own image, never part of the server image. It only fills a
  database with no accounts; to start over, delete the database volume
  (`docker compose down -v`). There is deliberately no reset option.
- It calls the server's own service functions, so demo data is validated and
  sanitized like real input.
- Seeded accounts share one documented password; the output lists them.
- Content comes from built-in word lists and is reproducible from a seed
  number. Nothing is downloaded.

Done when: a fresh `docker compose up` plus the seed command gives a board
with several pages of varied prompts, and every seeded account can sign in.

### 1d: Admin settings: tag management and required tag types

- An **Server settings** area in the board, linked in the header for admins
  only and refused (403) to everyone else. Later settings go here as well.
- **Tag types** (namespaces): list, create, rename the display name, lock or
  unlock user tags, and mark as **required**.
- **Required tag types**: every save, of a new prompt or an edit to an old
  one, must carry at least one tag of each type that is required at that
  moment. Existing prompts aren't changed when a type becomes required, but
  the next edit must add one. Changing only a prompt's open, busy or closed
  status doesn't count as an edit. The prompt form marks required types, and
  the error names the missing ones. The API and the CLI enforce the same rule.
- **Tags**: list with usage counts and search, create, rename, alias one tag
  to another, and delete. Deleting removes the tag from every prompt that
  uses it, after a confirmation page showing how many prompts that is.
- Every change is written to the admin log.

Done when: an admin can do all of the above in the browser without
JavaScript, non-admins get 403 on every settings URL, and the required-type
rule has end-to-end tests (new and edited prompts refused without the tag,
existing prompts left as they were, status changes still allowed).

As built: the board pages are under `/admin/settings` (tabs "Tag types" and
"Tags"). The JSON API adds `PATCH /admin/tag-namespaces/{name}`,
`GET /admin/tags` (with usage counts) and `GET`/`DELETE
/admin/tags/{namespace}/{slug}`, and the CLI has `admin namespace-set`,
`admin tags` and `admin delete-tag`. Adding a tag that is already curated is
refused (409) rather than logged twice. The admin log also refuses
`TRUNCATE`, which row triggers alone didn't stop.

### 1e: Tag moderation

- **New tags from writers**, one switch in Server settings, changeable at any
  time: *Allowed*, *Need approval* or *Not allowed*. It replaces the
  `QUILL_TAG_POLICY` variable, which now only sets the starting value
  (`open`, `approval` or `curated`; the old `user` and `both` mean `open`).
  A tag type marked curated-only still refuses new tags whatever the switch
  says. Each change is logged.
- **Hidden tags.** An administrator can hide any tag. It stays on its
  prompts but only administrators see it. For everyone else it isn't shown,
  isn't suggested, isn't listed, and filtering by it behaves exactly like
  filtering by a tag that doesn't exist, so nothing reveals it. An author
  editing a prompt can't see its hidden tags, so saving keeps them.
- **Tag requests.** With *Need approval*, a new tag a writer uses is created
  as *awaiting approval*, using the hidden mechanism. Its author sees it on
  their own prompt, marked as awaiting approval; nobody else does until it is
  approved. A "Tag requests" tab lists them, and the header shows how many
  are waiting. An administrator can:
  - **approve** it (visible to everyone),
  - **merge** it into an existing tag (its prompts move to that tag), or
  - **reject** it (removed from its prompts, like deleting).
  A tag awaiting approval still counts towards required tag types; if it is
  rejected, the prompt needs a new tag of that type at its next edit.
- API: `GET`/`PATCH /admin/settings`, a `status` on tags (`visible`,
  `hidden`, `pending`), a status filter on `GET /admin/tags`, and `status` in
  `PATCH /admin/tags/{namespace}/{slug}`. CLI commands to match.

Done when: each switch setting, hiding, and each request outcome have
end-to-end tests that check what anonymous visitors, the author, other
writers and administrators each see, including through filters.

### 1f: Tag picker and tag colours

- Chosen tags appear as removable chips, each with an ×. Removing is
  instant without JavaScript: each chip is a checkbox styled as a chip, and
  an unticked chip isn't sent.
- Tags are added by typing `type:name` (with the browser's own suggestion
  list) and pressing Add, or by choosing from "Browse tags", grouped by type.
  Either reloads the form with everything typed so far kept, and jumps back
  to the tags. Enter in a text field adds rather than posts, so a prompt is
  never posted by accident.
- Typed tags are checked as they're added: an unknown type, or a new tag the
  server or tag type doesn't accept, is refused with a message there and
  then. New tags show as "new" (or "new, needs approval").
- Required tag types show as empty slots ("Genre: required") until filled.
- Tags awaiting approval show as pending chips.
- **Colours.** Each tag type can have a chip colour pair (background, and
  text that is either chosen or automatic: black or white, whichever reads
  better). A tag can have its own pair, which replaces its type's. Chips
  everywhere use it: the picker, prompt cards and prompt pages. Colours are
  any `#rrggbb`, picked with the browser's own colour picker, with a preview
  and a warning below 4.5:1 contrast. The CSP forbids inline styles, so the
  server generates `/static/tag-colors.css` from the settings (decided
  2026-10-22, over a fixed palette). Only validated colours and `[a-z0-9-]`
  identifiers reach it, and only visible tags' own colours are included.
  Browsers revalidate it on each page (ETag). Colour changes are logged.
- The API's `Tag` carries its effective colours, text resolved;
  `TagNamespace` and `AdminTag.ownColor` carry them as set. CLI:
  `admin namespace(-set) --color --text-color`, `admin tag-color`.

Done when: tags can be added and removed on the new and edit forms with the
keyboard alone and without JavaScript; colours show in both themes and fall
back correctly from tag to type to none.

### 1g: Persona handle rules

- Handles are unique across the server, case-insensitive and ASCII only
  (enforced by the database and tested since Phase 0). They can't be
  renamed.
- **Reserved handles.** Writers can't create personas with them ("reserved
  for this server's staff"). The list is the administrators' (decided
  2026-10-22, not hard-coded):
  - It starts as Quill's defaults, seeded once: staff words (`admin`,
    `moderator`, `support`, `quill`, …) and exact names that could pass for
    the system or a page (`everyone`, `api`, `inbox`, …). Administrators may
    remove any of them.
  - Administrators add and remove entries in **Server settings → Handles**
    (one per line, with a tester showing which entry reserves a handle) or
    the API and CLI.
  - `QUILL_RESERVED_HANDLES_FILE` names a file synced at every start, for
    large or automated lists: one per line, a word unless it starts with
    `=`, `#` comments. Its entries can only change in the file; a bad line
    stops startup with line numbers. Syncs are logged as the server's.
  - A **word** reserves the handle that is the word, has it as a
    `_`-separated part (`site_admin`) or spells it with underscores removed
    (`ad_min`); `badminton` is fine. An **exact** entry reserves only
    itself. Digits are read as letters (`adm1n`). The server's own name is
    always reserved as a word.
  - Checks look candidates up in an indexed table, so long lists stay fast.
    Existing personas aren't affected.
  Only `quill-server grant-handle <login> <handle>` can create a persona
  with a reserved handle.
- **Retired handles.** Deleting a persona retires its handle for good, for
  everyone including its old owner and administrators, because other servers
  will have cached the name once federation exists. A database trigger
  records it, so every way a persona can be deleted is covered. Only the
  handle is kept, not who had it.
- A taken or retired handle gets the same reply, "That handle isn't
  available", so the reply doesn't reveal which.

Done when: tests cover reserved names (including case and digit variants and
the server's name), retired handles (same and other accounts, case variants,
cascading deletes) and granting.

### 1h: Account moderation notices

The foundation for moderation that people can see. Each account gets
**notices**: a record of every action an administrator took against its
prompts and tags, readable only by that account.

- What creates a notice today: a prompt removed after a report (with the
  report's category); a tag on the account's prompts deleted, rejected,
  merged into another, hidden, shown again or renamed; a tag the account's
  prompts carry approved. For tag actions, each affected account gets one
  notice listing only its own prompts.
- What doesn't: a discarded report (no action was taken, and telling people
  they were reported would chill writing and could expose reporters); colour
  changes (cosmetic).
- A notice names the action, the affected prompts (titles are kept, so the
  notice still reads after a prompt is deleted) and the persona each was
  posted as. It never names the reporter or the administrator.
- Each notice links to the admin-log entry it came from, so the public-facing
  record and the internal one can't drift apart.
- Unread notices are counted in the header ("Notices"); viewing them marks
  them read. They can't be deleted by the account; they go with the account.
- Notices are typed records with structured fields, not stored sentences, so
  later moderation (warnings, messages from administrators, suspensions,
  appeals) adds new kinds rather than reworking this. Clients render the
  text.
- API `GET /account/notices`, `POST /account/notices/read`; the unread count
  in `GET /account`. CLI `quill account notices`.

Done when: end-to-end tests show each action reaching exactly the affected
accounts, with nothing about reporters or administrators, and unread counts
behaving.

## Phase 2: Encrypted sessions on one server

| Sub-phase | Scope |
| --- | --- |
| 2a | Identity key and device certificates in `quill-core`; device key directory on the server |
| 2b | Device linking and the encrypted local store |
| 2c | KeyPackage upload and claim; certificate checks on claimed packages |
| 2d | Collaboration requests: request, approve, silent decline, 30-day expiry, one pending per prompt |
| 2e | Session creation and Welcome delivery; the epoch sequencer |
| 2f | Messages: ordering (HLC), history that is never lost, live delivery over SSE |
| 2g | Channels within a session |
| 2h | Attachments and size limits or quotas |
| 2i | Backup and recovery; safety numbers |
| 2j | Phase gate: the plaintext-canary test and the database-dump audit |

## Phase 3: Federation

| Sub-phase | Scope |
| --- | --- |
| 3a | Actors, WebFinger, HTTP Signatures |
| 3b | Board federation: follows, prompt delivery, remote prompts on the board |
| 3c | Remote collaboration requests and KeyPackage claims |
| 3d | Cross-server sessions sequenced by the prompt author's server |
| 3e | Two-instance test harness; interop with a real Mastodon instance |

## Phase 4: Hardening and review

| Sub-phase | Scope |
| --- | --- |
| 4a | Malicious-server test suite |
| 4b | Fuzzing of every untrusted-input parser |
| 4c | Operations runbook, disclosure policy, Tor onion service |
| 4d | External review and fixes |

## Phases 5 and 6

The desktop GUI and the web client get their own sub-phases when Phase 4 is
done.

## Decisions

- **1f, JavaScript on the board (decided 2026-10-22): none.** Adding a tag
  is a form button that reloads the form with the visitor's input kept and
  jumps back to the tag field. Removing a tag is instant without JavaScript:
  each chip is a checkbox styled as a badge with an ×. The CSP stays at
  `default-src 'none'` with no script source. A small same-origin script
  could be added later as an enhancement on top of this.
- **1d, required tag types (decided 2026-10-22):** they apply to every save,
  of new prompts and edits alike.
- **1e, tag moderation (decided 2026-10-22):** one live three-way switch for
  new tags; authors see their own pending tags; requests can be approved,
  merged or rejected; hidden tags can't be used in filters by non-admins.
