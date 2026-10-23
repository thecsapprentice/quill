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

## Phase 1: Prompt board

| Sub-phase | Scope | Status |
| --- | --- | --- |
| 1a | Prompts, tags, search, reports, admin inbox: API and CLI | done (#1) |
| 1b | Server-rendered board (`quill-board-ui`), sessions, CSRF | done (#2) |
| 1c | Docker setup and demo data | done |
| 1d | Admin settings: tag management and required tag types | done |
| 1e | Tag picker on the prompt form | next |
| 1f | Persona handle rules | planned |

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

### 1e: Tag picker on the prompt form

- Chosen tags appear as removable chips, each with an × button.
- Tags are added from a suggestion list or typed as `type:name`.
- Required tag types (from 1d) are shown as prompts to fill.
- No JavaScript: "Add" reloads the form; × is a checkbox chip. See Decisions below.

Done when: tags can be added and removed on the new and edit forms with the
keyboard alone, and the form still works with JavaScript disabled.

### 1f: Persona handle rules

- Handles are already unique across the server, case-insensitive and ASCII
  only (enforced by the database and tested).
- Add a reserved list (`admin`, `administrator`, `moderator`, `support`,
  `staff`, `system`, `root`, `quill`, the server's own name, and similar)
  that only `quill-server` admin commands can grant.
- Retire deleted handles permanently, so nobody can take over a deleted
  persona's name. Once federation exists, other servers will have cached
  that name.

Done when: tests cover reserved names, retired handles and case variants.

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

- **1e, JavaScript on the board (decided 2026-10-22): none.** Adding a tag
  is a form button that reloads the form with the visitor's input kept and
  jumps back to the tag field. Removing a tag is instant without JavaScript:
  each chip is a checkbox styled as a badge with an ×. The CSP stays at
  `default-src 'none'` with no script source. A small same-origin script
  could be added later as an enhancement on top of this.
- **1d, required tag types (decided 2026-10-22):** they apply to every save,
  of new prompts and edits alike.
