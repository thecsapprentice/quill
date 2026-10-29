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

Admin tools added since, tracked as issues under the Phase 1 epic (#34):
announcements and comments (#41), automated moderation (#43), the rules
page (#42), hold-for-review (#44) and shared script code (#47).

### Automated moderation (#43)

Administrators' scripts check prompts, edits, comments, new tags and
reports before they're saved, and can refuse, flag or dismiss. Scripts are
in Rhai with a native `text` API, read shared common data, run in an
ordered chain per hook with an error setting, and start from seven
templates. Admin pages under Server settings → Moderation, the
`/admin/moderation` API and `quill admin automod`. Design: the
architecture document, "Automated moderation"; reference for admins:
[automod.md](automod.md).

### 1c: Docker setup and demo data

- `Dockerfile` and `compose.yaml` for local testing (done).
- A demo-data seeder, run by `docker compose --profile demo run --build --rm seed`:
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
- **Accountability in the admin log** (administrators only). Every entry
  records who acted (an administrator, by login name as it was; the server
  itself; or a writer's report, whose writer stays unnamed) and which
  accounts the decision was against, by login name as it was (none for
  server-wide changes). Administrators filter the log by either, so they
  can see what each of them is doing and when. Both records stay
  append-only and survive account deletion. Existing entries were
  backfilled from notices and reported prompts. API `GET
  /admin/log?admin=&account=`; CLI `quill admin log --admin --account`.
- **Long lists of affected accounts.** One decision can reach thousands of
  accounts (deleting a common tag), so log listings name the first five
  (an account being filtered for comes first) and link "and N more" to the
  entry's own page. That page lists every account alphabetically, 100 at a
  time, each with the prompts the decision touched as they were then, and
  whether each prompt is still on the server. Notices for such a decision
  are written in batches. API `GET /admin/log/{id}?after=&limit=`; CLI
  `quill admin log-entry ID --after`.

Done when: end-to-end tests show each action reaching exactly the affected
accounts, with nothing about reporters or administrators, and unread counts
behaving.

## Phase 2: Encrypted sessions on one server

Done (2026-10-23): the gate passed in 2l and the conformance artifacts
followed in 2m.

Reordered on 2026-10-22 (see Decisions): the encrypted local store comes
first, so private keys never touch disk unprotected, and OpenMLS arrives
before device linking, which needs it.

| Sub-phase | Scope | Status |
| --- | --- | --- |
| 2a | Encrypted local store in `quill-core`; the CLI's login moves into it | done (#9) |
| 2b | Identity keys per persona, device keys and certificates; the server's device directory | done (#10) |
| 2c | OpenMLS on the client; KeyPackage upload and claim; certificate checks on claimed packages | done (#11) |
| 2d | Device linking and the personal device group, on the group message log and epoch sequencer | done (#12) |
| 2e | Collaboration requests: request, approve, silent decline, 30-day expiry, one pending per prompt | done (#13) |
| 2f | Sessions after acceptance: the session list, Welcomes for members' other and later devices, writing to the log, Leave | done (#14) |
| 2g | Messages: ordering (HLC), history that is never lost, live delivery over SSE | done (#15) |
| 2h | Channels within a session | done (#16) |
| 2i | Attachments and size limits or quotas | done (#17) |
| 2j | Safety numbers; backup and recovery; rejoining sessions after a restore | done (#18) |
| 2k | History sync for devices added later; deleting messages by age | done (#19) |
| 2l | Phase gate: the plaintext-canary test and the database-dump audit | done (#20) |
| 2m | Conformance artifacts: the OpenAPI file, JSON Schemas and the remaining test vectors | done (#21) |

### 2a: Encrypted local store

Everything a client keeps on disk lives in one encrypted file; later
sub-phases add their tables (keys, trust pins, MLS state, history).

- **Interface first.** `quill-core`'s `Store` has typed methods, built on a
  small storage interface: named tables of byte keys and values, point
  reads, prefix scans and atomic batches. Nothing else touches the file.
  Every backend must pass one shared conformance test, so the backend can be
  replaced without touching callers (see Decisions for the cost).
- **Backend: redb with page encryption.** redb (pure Rust) stores its data
  through our storage layer, which seals each 4 KiB page with
  XChaCha20-Poly1305 under a key derived from the store's random data key.
  Each page's slot is bound to the file, the page number and a write
  counter. Every page has two slots on disk and writes alternate between
  them, so a crash mid-write leaves the page as it was. A thief sees the
  file's size and, across copies, which pages changed; not tables, keys,
  values or their sizes. Replaying an old copy of a page is not detected,
  as with SQLCipher.
- **Unlocking.** The data key is wrapped in the file's header by a key from
  the OS keychain (Windows Credential Manager, macOS Keychain, the Secret
  Service on Linux desktops), or by a passphrase through Argon2id (64 MiB, 3
  passes, 4 lanes; at least 12 characters). The header is kept twice so a
  crash while changing it can't lock the store. Changing the passphrase or
  moving to the keychain rewraps the data key; it doesn't re-encrypt, so an
  older copy of the file still opens with the old passphrase.
- **One process at a time**: the file is locked while open.
- **CLI.** `quill` opens the store on every command, creating it on first use
  in the keychain if the computer has one, otherwise with a passphrase.
  `QUILL_STORE` and `--store` choose the file; `QUILL_STORE_PASSPHRASE`
  supplies the passphrase for scripts and tests. The server address and
  login token move into the store, and the old plain `profile.json` is
  imported and deleted. `quill store status`, `quill store passphrase` and
  `quill store keychain`.

Done when: tests show the file holds no plaintext, refuses wrong keys and
altered pages, survives torn page and header writes and a crash at every
point of a commit, and the CLI works end to end from the store.

### 2b: Identity keys per persona, device certificates, device directory

Each persona that will take part in sessions gets keys; the server publishes
the public halves so other writers' clients can check them. Nothing here
encrypts yet: 2c uses these keys for MLS.

- **Actor URIs.** Every persona gets its permanent identifier,
  `https://{domain}/users/{handle}`, shown in the personas API. Handles are
  never renamed or reused, so it's stable. Certificates sign it, so changing
  the server's domain later invalidates them (fine before federation, Phase 5).
- **Keys, all Ed25519 and generated on the client:**
  - an **identity key** per persona, with a version that goes up when it's
    replaced;
  - a **device key** per persona on each device (one device per persona
    until linking arrives in 2d). It becomes the MLS signature key in 2c.
  - Private halves live only in the encrypted store.
- **Formats** (new crate `quill-certs`, linked by both client and server):
  `DeviceCertificate` and `DeviceRevocation` exactly as in the protocol
  draft, TLS-encoded, signed with MLS's `SignWithLabel` (labels
  `"MLS 1.0 wr device cert v1"` and `"MLS 1.0 wr device revoke v1"`).
  Verification is strict: exact lengths, no trailing bytes, unknown flag
  bits refused, Ed25519 strict verification. Signing is client-only.
- **Test vectors** in `test-vectors/`, generated from fixed seeds and
  checked by the tests: labeled signatures, certificates and revocations,
  each with the must-reject cases from the protocol draft.
- **Server: the device directory.** The server checks certificates and
  revocations itself (public-key operations only, which the dependency ban
  allows) and refuses anything that wouldn't verify; clients check again
  and never take the server's word.
  - `PUT /personas/{id}/identity-key` sets or replaces the key; a
    replacement needs a higher version and leaves the persona's existing
    devices uncertified until each is certified again.
  - `POST /personas/{id}/devices` registers a certified device;
    `POST /personas/{id}/devices/{device}/revoke` takes a signed
    revocation, which is permanent.
  - `GET /actors/{handle}/keys` is public: the identity key, its version,
    and every device with its certificate, plus revocations.
- **Unlinkability.** Device ids are random (not time-ordered), the server
  publishes no creation times, and clients round a certificate's start time
  down to the UTC day, so devices certified together for different personas
  can't be matched up by timestamps.
- **Trust on first use.** The store pins the first identity key seen for
  each persona; a later different key is reported as a change, never
  accepted silently. Safety numbers come in 2j.
- **CLI:**
  - `quill keys init --as HANDLE` creates and publishes keys.
  - `quill keys show HANDLE` fetches any persona's keys, checks every
    certificate, and reports its fingerprint and pin status.
  - `quill device list --as HANDLE` and
    `quill device revoke DEVICE --as HANDLE [--reason lost|compromised|retired]`.
  - On a second computer, `keys init` refuses to replace a persona's key
    unless given `--new-identity`, which warns that other devices stop
    working until linked (2d).

Done when: the test vectors pass; the server refuses every bad certificate
and revocation; a client detects a forged device or a swapped identity key
even when the server accepts it; and a persona's keys reveal nothing that
links it to the login's other personas.

### 2c: OpenMLS, KeyPackages and claims

A writer's client can now be added to an MLS group while they're offline: each
device publishes KeyPackages, and anyone can claim one per device of a
persona and check it. Groups themselves come in 2f.

- **OpenMLS 0.9 on the client only**, with the RustCrypto provider and
  ciphersuite `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`. The server's
  dependency ban keeps it out of the server.
- **MLS state in the encrypted store.** OpenMLS's storage interface is
  implemented over the store (adapted from its in-memory storage, MIT), with
  batches so the many writes of one MLS operation land together or not at
  all.
- **KeyPackages.** Credential: MLS `BasicCredential` whose identity is the
  persona's actor URI. Signature key: this device's key for the persona
  (2b). Leaf extension `0xF0A1` carries the device certificate, and the
  leaf's capabilities list it so groups can require it. Lifetimes run from
  the day before to 28 days after the UTC day they're made, so packages
  made together for different personas share no telling timestamps.
- **The pool.** Each device keeps up to 20 one-time KeyPackages per persona
  on the server, plus one last-resort package (reused only when the others
  run out, replaced when it nears expiry).
  - `quill keys publish --as HANDLE [--count N]` tops it up.
  - `quill keys status --as HANDLE` shows what's left.
  - The server holds at most 100 one-time packages per device.
- **Server checks** (structure only, no MLS library): `quill-mls-envelope`
  parses a KeyPackage just far enough to check that it's MLS 1.0, our
  ciphersuite, a basic credential naming this persona's actor URI, signed
  by this registered, unrevoked, currently certified device key, carries a
  certificate extension, and is in date. Signatures are for clients to
  verify.
- **Claims.** `POST /actors/{handle}/key-packages/claim` (signed in, rate
  limited per account) returns one package per current device, removing
  one-time packages as it hands them out and falling back to the
  last-resort one. Expired packages are dropped.
- **Checking a claim.** The claimer's client accepts a package only if
  OpenMLS validates it (signatures, lifetime), its credential names the
  persona, its signature key is a device the directory lists as valid, and
  its certificate extension verifies under the persona's pinned identity
  key. A changed identity key stops the claim until the user accepts it.
  Anything else is reported and never used.

Done when: a claimed package from every device of a persona passes the
checks; tampered, substituted or uncertified packages are refused by the
claimer even when the server hands them out; the pool drains to the
last-resort package and refills; and MLS state survives reopening the store.

### 2d: Device linking and the personal device group

A writer's second computer joins their personas: it receives the
personas' identity keys from a device they already use, over an MLS group
that only their own devices belong to. Decided 2026-10-23: the group
message log and epoch sequencer are built here (moved from 2f), and only
full devices are linked; limited devices wait for the web client.

- **Group message log (any MLS group).** The server keeps an ordered log
  per group and reads only the clear header of each `PrivateMessage`
  (`quill-mls-envelope`: group id, epoch, content type). The first commit
  for the current epoch wins and advances it; a late commit, or a message
  for an old epoch, gets `409` and the client catches up and retries.
  Groups use ciphertext only (no `PublicMessage`), so commits are as opaque
  as text. 2f adds session groups on top.
- **The personal device group.** One per login, holding only its devices.
  Each device joins it with a separate **account device key**, never
  published and tied to no persona, so the group says nothing about
  personas. The server knows the group belongs to the login, as it already
  knows the login's personas.
- **Linking**, following the protocol draft:
  1. On the new device, signed in to the same login: `quill device link`.
     It makes its account device key and a KeyPackage, posts them to a
     mailbox that expires in 10 minutes, and shows a code:
     `quill-link:1:HOST:LINK_ID:FINGERPRINT`, plus 6 words from the
     fingerprint (BIP-39 English).
  2. On a device already in use: `quill device approve CODE` (or the link
     id and the 6 words). It checks the KeyPackage against the fingerprint,
     so the server can't swap in its own, adds the new device to the
     personal group, and sends it, inside the group, every persona's
     identity key and the trust pins.
  3. Both devices then show 6 confirmation words derived from the group's
     epoch authenticator. They match only if both are in the same group, so
     the server can't hand the new device a group of its own making. The
     new device keeps the keys only after the user confirms.
  4. The new device certifies its own device key for each persona,
     registers it and fills its KeyPackage pool, as `keys init` does.
- **Staying in sync.** Every command catches up on the personal group
  first: new devices, removals, and keys granted after an identity key was
  replaced elsewhere. Each device announces in the group which persona
  devices are its own.
- **Keeping KeyPackage pools full.** Until now pools were refilled only by
  `keys init` and `keys publish`. Now every command that talks to the
  server first checks this device's pool for each persona (one request):
  below 10 one-time packages it refills to 20, and it renews the
  last-resort package within 7 days of expiry. In 2f, a device that joined
  a session through its last-resort package sends a key update on its next
  run; in 2g, the server tells open clients over the live event stream
  when their pool runs low.
- **Removing a device:** `quill device remove ID` takes it out of the group
  (rotating the group's keys) and revokes each of its persona devices. A
  removed device that catches up deletes the identity keys it held.
- **CLI:** `quill device link`, `quill device approve`, `quill devices`
  (the login's devices and their persona devices), `quill device remove`.

Done when: a second and third device link in any order and all three stay
in sync; a substituted KeyPackage or a server-made group is caught by the
fingerprint or the confirmation words; two devices committing at once both
end up consistent; and a removed device can read nothing sent after.

### 2e: Collaboration requests

A writer asks to join a prompt; its author reads the pitch and accepts,
declines, or lets it lapse. The request is cryptographic from the start
(protocol draft, section 4): the requester's client builds the session's
MLS group and encrypts the pitch inside it, so the server files and
forwards only ciphertext. Acceptance turns the group into a session whose
log the server sequences, on 2d's group message log. Everything after
acceptance (the session list, Welcomes for devices that weren't in the
request, writing, Leave) is 2f.

- **Sending** (`quill request send PROMPT --as HANDLE`):
  1. Claim one KeyPackage per device of the prompt's author (2c's checks:
     directory, certificates, the TOFU pin). With no usable package, stop.
  2. Claim KeyPackages for the requesting persona's other devices, if it
     has any, so they're in the group from the start.
  3. Create the MLS group (random 32-byte id), add all of them, and encrypt
     the pitch as the group's first application message: an AS2 `Create`
     of a `Note` in the `planning` channel.
  4. `POST /requests` with the Welcome, the pitch and an optional expiry in
     days (at most 30). The commit that added everyone goes nowhere: every
     member joins from the Welcome.
- **The server files it** by the prompt's state when it arrives: `open`
  makes it pending, `busy` holds it, `closed` refuses it. A refused request
  is stored without its ciphertext and never shown; the requester gets the
  same answer as for any other. A prompt going back to `open` releases held
  requests; reading the held queue releases them too. Closing a prompt
  keeps what's already filed.
- **One waiting request per login per prompt**, across all its personas:
  pending, held, refused and declined requests all count until they expire,
  so a second request can't reveal a decline. Withdrawing frees the slot.
  New requests are limited to 20 per login per hour.
- **What each side sees.** The requester sees only *waiting*, *session*
  (accepted), or *no response* (after expiry). The author sees pending
  requests (and how many are held) with the requester's persona and the
  ciphertext, which their client decrypts.
- **Reading a request** (`quill request inbox PROMPT [--held]`): the
  author's device joins the group from the Welcome and checks every member
  against the directory: each must be a valid device of the requester or of
  the author's persona, and the pitch must come from one of the
  requester's. A group that fails is dropped and reported, never shown as
  genuine. The pitch is kept in the local store, since MLS decrypts each
  message only once. A device that wasn't in the request (linked later)
  says so; 2f brings it in after acceptance.
- **Accepting** (`quill request accept ID`, after reading it): the server
  registers the session group at the pitch's epoch, puts the pitch at the
  head of its log, and records the two personas as its members, so both
  logins (and only they) can read and post to it.
- **Declining** (`quill request decline ID`) tells the requester nothing:
  the server keeps a ciphertext-free record until expiry, so the slot
  stays taken. The author's devices drop the group.
- **Withdrawing** (`quill request withdraw ID`) deletes a request that isn't
  a session yet; `quill request list` shows the login's requests.
- **Expiry:** the earlier of the requester's choice and 30 days. Deleting a
  prompt, or the author's persona or login, or an administrator removing
  the prompt after a report, expires its waiting requests at once, so the requester sees *no response* rather than the request
  vanishing. Expired requests lose their
  ciphertext; the requester's view keeps *no response* until dismissed.
- **Cleaning up.** Each device notes the requests it holds a group for.
  The upkeep before each command checks those still undecided and drops
  the group of any that was declined, withdrawn or expired.
- **Prompts** now report `acceptingSessions`: the author has an identity
  key and a full device with KeyPackages.

Not in 2e: what administrator action does to sessions already running (they
carry on) and account suspension are to be decided closer to a working
implementation. Blocking doesn't exist yet. When it's built, a blocked login's
requests are filed as refused, silently, as the plan describes.

Done when: a request reaches only the author's devices; held, refused and
declined requests all look like *waiting* to the requester; a second
request from any persona of the same login is refused until the first
expires or is withdrawn; an accepted request becomes a group log both
sides can use; and a forged member or pitch sender is caught by the author's
client.

### 2f: Sessions

An accepted request is a session: two writers (the prompt's author and the
requester), each with one or more devices, sharing an MLS group whose log
the author's server sequences. 2f makes it usable: every device of both
writers joins, they write and read, and either can leave. Ordering by the
writers' clocks, edits and deletions, and live delivery are 2g.

- **The session list** (`GET /sessions`, `quill sessions`): the login's
  sessions, each with its group id, both personas, the prompt (while it
  exists), its place in the log, whether it has ended, and the Welcomes
  waiting for this login's devices. The session id is the group id; the
  CLI accepts any unambiguous prefix of it.
- **Every device joins.** Each command's upkeep syncs every session:
  - A device that holds the group already (the requester's, or an author
    device that read the pitch) carries on from its note of the request.
  - A device that was in the request's Welcome but never opened it (the
    requester's other devices; author devices that didn't read the pitch)
    joins from that Welcome, which the server keeps with the session, and
    reads the log from the start, pitch included.
  - A device that wasn't in it (linked later, or certified again after an
    identity key change) is added by one of its own persona's devices
    already in the group: that device claims a KeyPackage for just the
    missing device, commits the add, and leaves the Welcome on the server
    addressed to it. Whichever device commits first wins the epoch race;
    the others catch up and find nothing left to do.
  - A device of one's own persona that the directory shows revoked is
    removed the same way.
  - A device that joined through its last-resort KeyPackage sends a key
    update (a self-update commit) straight away, so that key isn't reused.
- **Checks on every change**, by every device, against the directory and
  TOFU pins:
  - joining: each member is a valid device of one of the two personas (as
    in 2e);
  - a commit adding a device: the device is a valid device of the
    *committer's* persona. A writer only ever adds their own devices, so
    neither writer, nor the server, can slip a device into the other's
    name;
  - a commit removing a device: it's the committer's own persona's device.
  A change that fails is refused: the device stops reading the session and
  reports why, rather than going on in a group it can't trust.
- **Writing and reading** (`quill session write ID`, `quill session read
  ID`): messages are AS2 `Create`s of Markdown `Note`s, in the `story`
  channel unless another is given, encrypted to the group and appended to
  the log. Each device decrypts the log in order and keeps what it read in
  the encrypted store (MLS decrypts a message only once, and a device can't
  decrypt its own), so reading is local. A device added later reads from
  when it joined; sharing earlier history with it is 2j's history bundles.
- **Leave** (`quill session leave ID`): with two writers, one leaving ends
  the session. The server marks it ended and accepts nothing more for its
  log; both sides can still read what's there, and their devices drop the
  group's keys once they've read to the end. Deleting either persona (or
  its login) ends its sessions too. Keeping or deleting an ended session's
  log on the server is left to 2g's delivery acknowledgements.
- **Late messages** (the "based-on seq" idea from 2d): unchanged. The
  sequencer already refuses a message for any epoch but the current one,
  and readers keep 5 past epochs, so a message is never stored where its
  readers can't decrypt it. 2g revisits this if live delivery shows a need.

Done when: both writers' devices, including ones linked after the request
and ones that never read the pitch, all join and read every message sent
after they joined; a device added under the wrong persona, or a member that
isn't a valid device, is refused; two devices racing to add the same
missing device end up consistent; a revoked device is removed and can't
read what follows; and leaving ends the session for both.

### 2g: Messages

2f's messages are plain notes read in log order. 2g gives them the
protocol draft's shape (section 5): writers' clocks decide the order on
screen, edits and deletions keep every version, new messages arrive live,
and the server learns when it may let go of ciphertext.

- **Ordering.** Every message carries a hybrid logical clock value (`hlc`:
  wall-clock milliseconds, a hyphen, a 4-digit counter) and an `authorSeq`.
  Each channel shows its messages by `hlc`, then sender, then `authorSeq`.
  The clock moves forward past every value a device reads, so a reply
  always sorts after what it answers, even with a skewed clock or a draft
  written offline. A value more than a day ahead of the reader's own clock
  doesn't move its clock, so one bad clock can't drag everyone's forward.
- **`authorSeq` counts per sending device**, not per writer: a writer's
  devices send independently and can't share a counter without talking
  first. A gap in a device's counter (between the first and last values
  seen) means the server held messages back; `quill session read` says so.
- **Edits and deletions keep every version.** An `Update` replaces a note's
  text and a `Delete` hides it, both as new encrypted activities appended to
  the log; the original stays. Only the original author's (the same
  persona's) `Update` or `Delete` counts. Reading shows each message's
  latest text, marked edited, or "deleted by author"; `quill session
  history` shows every version. The server can't tell an edit from a new
  message.
- **Padding.** Application messages are padded to a multiple of 256 bytes
  before encryption, so ciphertext sizes say less about what was written.
- **Live delivery** (`GET /events`, server-sent events): an authenticated
  stream of small notices for the login: a session or device-group message
  was logged (group and position), a request is waiting on a prompt, a
  session began or ended, and a device's KeyPackage pool ran low. Notices
  carry no content; clients fetch and decrypt as before.
  `quill session read ID --follow` stays open and prints new messages as
  they arrive.
- **Acknowledgements and retention.** Each device tells the server how far
  it has read a session's log (`POST /sessions/{id}/ack`). These go only to
  the server, never to the other writer. The server deletes a message once
  every current device of both personas (valid in the directory) has
  acknowledged it and a grace period of 7 days has passed. A device that
  hasn't acknowledged anything holds everything, so nothing is deleted
  before a device that will need it has read it. Deleting by age alone, for
  devices that never come back, waits for 2k's history sync.

Not in 2g: cover traffic (optional `IntransitiveActivity` messages), and
`inReplyTo` and channel management, which are 2h.

Done when: messages from devices with skewed clocks sort causally; a
withheld message shows as a gap; edits and deletions show their latest
state, keep every version, and are ignored when they come from the other
writer; a second client sees a message within a second of its being sent;
and acknowledged messages are deleted from the server only after every
device has them.

### 2h: Channels

A session is one MLS group and one log; channels live entirely inside the
encrypted activities (plan: channel tag in the payload), so the server
can't tell planning from story, or how many channels a session has. 2g
already carries a `channel` on every activity; 2h makes channels something
writers can see, name, and move between. Nothing changes on the server.

- **Names.** `story` and `planning` always exist. Other names follow slug
  rules: lowercase letters, digits and single hyphens, 1 to 32 characters.
  A device refuses to send to a name that breaks them; one received from
  the other writer's client is filed under `story`, with the note kept as
  sent.
- **Naming a channel** (`quill session channel add ID NAME [--about
  TEXT]`): an encrypted `Create` of a `wr:Channel` object, so the channel
  shows for both writers before anything is written in it. `--about`
  again replaces the description (an `Update`), keeping earlier versions
  as for notes. Either writer can describe any channel. Channels are never
  deleted, since nothing in a session is lost; writing in one names it
  too.
- **Replies across channels** (`quill session write ID --reply-to N`): a
  note can carry `inReplyTo`, the id of another note in any channel ("see
  the planning note"). Reading shows what a note answers and where it is;
  a reply to a note this device never received says so.
- **Each channel in its own order:** `quill session read ID --channel
  NAME` shows one channel by its clock order, as 2g does for the whole
  session.
- **The channel list** (`quill session channels ID`): every channel with
  its description, how many notes it has, and how many this device hasn't
  shown yet. That count is kept only on the device: no read status ever
  reaches the other writer or the server.

Done when: both writers see a named channel before anything is written in
it; notes sort within their channel; a reply links across channels; a
malformed channel name can't create a channel; and the server's view of
a session (its log and its notices) is the same with one channel or many.

### 2i: Attachments, limits and quotas

Writers can attach files to messages. Decided 2026-10-23: any file type,
with size caps and nothing else. The server can't see what a file is, and
any type check a client could make is defeated by renaming. Cleaning images
(stripping metadata such as GPS, re-encoding) and showing them wait for the
GUI, which is the first client that renders anything; 2i makes sure
messages can carry attachments that a later client can show.

- **Encrypted blobs** (plan: encrypted blob plus key in the message). The
  sending device encrypts each file with a fresh 256-bit key (AES-256-GCM,
  one shot), pads the ciphertext to a size bucket, and uploads it to the
  session (`POST /sessions/{id}/blobs`). The server returns an id and a
  random capability token, keeping only the token's hash.
- **The message carries the rest.** The note's `attachment` list holds an
  `EncryptedAttachment` for each file (protocol draft, section 5): its URL,
  token, key, nonce, plaintext SHA-256, size, name and media type. The
  media type is only a label from the file name; nothing checks it. The
  server sees a blob of a bucketed size and nothing else.
- **Size buckets:** powers of two from 4 KiB up to the attachment limit
  (10 MiB). The server refuses any other size, so it learns only the
  bucket.
- **Fetching:** `GET /blobs/{id}` needs a login in the session and the
  capability token. Each device fetches, decrypts and checks every
  attachment as it reads the message that carries it, keeps the file in its
  encrypted store, and only then acknowledges the message. So 2g's
  retention also covers blobs.
- **Retention:** posting a message names the blobs it carries, binding
  them to its log position. A blob goes when its message does: every
  device has acknowledged it and the grace period has passed. A blob no
  message names is deleted after a day.
- **Limits and quotas:**
  - Requests: the attachment limit is enforced while the body is read, so
    an oversized upload is cut off without being buffered.
  - Quota: each login may store up to 500 MiB of blobs at once.
  - Rate: 30 messages per minute per login.
  - Clients refuse to fetch or decrypt more than the server's advertised
    attachment limit.
- **CLI:** `quill session write ID --attach FILE` (repeatable); `read`
  lists each message's attachments; `quill session save ID N --to DIR`
  writes them out.
- **An administrators' switch** (added 2026-10-23 at the user's request):
  attachments in sessions can be turned off, as a live server setting
  (starting value `QUILL_ATTACHMENTS`), in Server settings → Sessions or
  `quill admin settings --attachments off`. Uploads are then refused and the
  instance info says so, so clients refuse before encrypting anything.
  Together with the size limits it tells writers what the server is for.
  It can't stop a determined client from packing small files into
  messages, but the server can tell uploads apart, so it holds for blobs.
- **Storage views** (added 2026-10-23): a login sees what its sessions take
  (`GET /account/storage`, `quill storage`): in total, per session
  (messages, attachments, and the part it uploaded against its quota), and
  its device-sync log. Administrators see every login's totals
  (`GET /admin/storage`, `quill admin storage`, and the Sessions tab), and
  one login's sessions without persona handles, so the view doesn't link
  personas to logins. Sizes and counts of ciphertext only.

Not in 2i: inline base64 images, image cleaning and rendering (GUI), and
fetching blobs through one's own server from another (Phase 5).

Done when: a file attached on one device arrives intact on every device of
both writers; a changed byte is caught; the server sees only bucketed
sizes; a blob can't be fetched without its token or by a login outside the
session; quotas and limits refuse what's over them; and a blob is deleted
with the message that carries it.

### 2j: Safety numbers, backup and recovery

Two protections the plan puts in Phase 2: a way for writers to check that
nobody else is in their session, and a way back when every device is lost.
Decided 2026-10-23: history sync for devices added later (and with it,
deleting messages by age) is split out to 2k; backups use the `age` file
format through the `age` crate, behind a small interface so our own
implementation of the format could replace the crate later without the
rest of the code changing.

- **Safety numbers** (`quill session verify ID`), plan: defending against
  key substitution.
  - The **identity number**: 12 words from both personas' identity keys
    (each with its actor URI, in a fixed order). It changes only if either
    writer's identity key changes, so adding devices doesn't disturb it.
    Comparing it with the other writer out of band proves the server hasn't
    swapped either persona's key.
  - The **session check**: 6 words from the group's current epoch
    authenticator, with the epoch number. Two devices at the same epoch
    show the same words only if they're in the same group, so no hidden
    member and no man in the middle.
  - `--confirm` marks the other writer verified on this device, tied to
    their identity key: if the key changes, the mark goes and the TOFU
    warning shows. `quill sessions` shows who's verified.
- **Backup** (`quill backup create`, plan: recovery when no device is
  left). The client makes a recovery code (128 random bits, shown once as
  12 BIP-39 words with a checksum) and uploads an encrypted backup:
  - **Contents:** every persona's identity key, the trust pins and
    verification marks, and each session's history as this device keeps it
    (notes with every version, channels, and attachments). Never MLS
    state: a restored device joins as a new device.
  - **Format:** an `age` file with the scrypt passphrase recipient, the
    recovery words as the passphrase, so a backup can be opened with the
    standard `age` tools even without Quill. Inside, JSON (the draft said
    CBOR; JSON is what everything else uses).
  - **Storage:** `PUT /backup/{part}` in parts of up to 8 MiB, replacing
    the previous backup as a whole; `GET /backup`, `DELETE /backup`. Up to
    256 MiB per login, counted apart from the attachment quota. The device
    keeps the code in its encrypted store, so `quill backup create` again
    refreshes the backup without a new code (`--new-code` makes one).
- **Restore** (`quill backup restore`, on a new device signed in to the
  login): enter the 12 words; the device takes the identity keys, pins and
  history, makes its own device key for each persona and publishes it. It
  assumes every other device is gone: it revokes the personas' other
  devices (after asking), and starts a new personal device group in place
  of the old one.
- **Rejoining sessions.** A restored device usually has none of its own
  devices left in its sessions to add it, so it asks the other writer's
  client: a **rejoin request** (protocol draft, section 6), signed with the
  persona's identity key, carrying its KeyPackage and certificate and
  listing the persona's lost devices to remove. The server keeps it with the
  session until it's used. The other writer's client checks the signature
  against the pinned identity key and the device against the directory,
  then commits the add and the removals with the signed request in the
  commit's authenticated data, so every member can check it; this is the
  one case where a writer adds a device that isn't their own. It leaves the
  Welcome for the restored device, which reads on from there; what came
  before is in the backup.

Not in 2j: incremental backups (each backup replaces the last whole), and
history for devices added later (2k).

As built: the `age` crate is 0.11, not 0.12. 0.12's post-quantum recipient
needs a release of `ml-kem` that conflicts with the one OpenMLS's HPKE
uses. The crate's build-time localisation macros bring an unmaintained
TOML parser (RUSTSEC-2026-0374), ignored with the reason recorded. Our own
implementation of the format would remove both.

Done when: both writers' devices show the same safety numbers, and a
server-swapped identity key changes them; a backup restored on a new device
with only the 12 words brings back the personas and history, a wrong or
mistyped code is caught, and the backup opens with the `age` tool; and the
restored device rejoins its sessions through the other writer, who refuses
a rejoin request that isn't signed by the persona's identity key.

### 2k: History sync and deleting by age

A device added to a session after it began reads only from the commit that
added it. 2k gives it the rest from the login's other devices, and with
that in place the server can delete ciphertext by age, not only once every
device has it: a device that was away too long catches up from its other
devices instead of from the server.

- **History requests and bundles** (plan: linking a new device; protocol
  draft: `HistoryBundle`), over the personal device group.
  - Each device notes, per session, the log positions it has no history
    for: up to the commit that added it (less what a restored backup
    already holds), or the stretch it missed while away. While that's not
    empty it sends one `HistoryRequest` (the session and the range) to the
    device group.
  - Another device of the login answers once it has read past the range
    and has no gap of its own in it. Its `HistoryBundle` is addressed to
    the asking device and carries the notes with a version in the range
    (every version), the channels and the `authorSeq` counts, and each file
    those notes carry. The rows travel as encrypted blobs in the personal
    group (the attachment mechanism, at most 10 per message, more messages
    for more), each file re-sealed as its own blob, so nothing is limited
    by the message size.
  - The asking device merges: notes it lacks are added; notes it has gain
    the versions they lack (a version is never dropped); `authorSeq`
    counts are joined. Then it marks the range as had, and deletes the
    bundle's blobs, which only it needed.
  - Sessions that ended before a device was linked are listed too; the
    device asks for their whole history the same way.
  - `quill sessions` shows a session whose history is still incomplete.
- **Deleting by age** (plan: retention). The server deletes every
  session's and the device group's ciphertext once it's older than the
  retention period (published as `retentionDays`, 30 days by default,
  `QUILL_RETENTION_DAYS`), and with it the blobs it carried, whatever the
  acknowledgements say. `QUILL_RETENTION_DAYS=-1` turns this off: messages
  then stay until every device has read them, however old, and
  `retentionDays` is published as `null`. Deleting once every device has acknowledged, after
  the 7-day grace, carries on as before. A sweep runs hourly.
- **Devices that fell behind** (plan: idle devices).
  - The server records how far each group's log has been deleted. Asking
    for messages from before that point gets `410 Gone`: the device can't
    follow the group's keys any more.
  - **In a session**, the device drops its keys for it (keeping the
    history), and asks to be let back in: through another of the persona's
    devices, which adds it back as for any device that's missing, or, if
    it's the persona's only device, through a rejoin request (2j) to the
    other writer, removing its old place and adding the new one in one
    commit. It then asks its other devices for what it missed.
  - The persona's other devices remove a device that fell behind from
    their sessions, so an idle device holds no current keys; the server
    lists them with the session. A device the server has left a Welcome
    for counts as having read up to it.
  - **In the device group**, the device must be linked again
    (`quill device link`), with a new account device key; its persona
    keys and sessions stay. `quill device remove` of its old entry
    doesn't revoke persona devices another device still uses.

As built: a request nobody answers is sent again after 7 days and dropped
after 30; a device that fell behind waits a day for the persona's other
devices before asking the other writer (at once when it has none). While a
device's history is incomplete, `quill session read` says so instead of
warning about gaps. An edit read live for a note the device doesn't have
yet is dropped; the bundle brings the note with its versions as the
answering device holds them. The server counts a device that never
acknowledged anything as having read nothing when listing devices that
fell behind.

Not in 2k: answering history requests from the other writer's devices
(they ask only their own), deleting by age in a client's own history, and
removing the device-group entry of a device that never comes back
(`quill device remove` does it by hand).

Done when: a device added to a session after many messages, edits and
files, and a device linked after a session ended, both end up holding the
same history as the device that was there from the start; ciphertext
older than the retention period is gone from the database with its blobs;
and a device away longer than that rejoins its sessions (through a sibling,
or through the other writer when it has none) and gets back what it
missed.

### 2l: Phase gate

Phase 2's exit gate ([plan](plan.md#roadmap-and-testing-strategy)): the database holds no
plaintext or private keys, and uncertified devices are rejected. 2l proves
the first with two checks that stay in CI, so every later change is held to
it; the second is already proven by 2b and 2c's tests, listed below.
Decided 2026-10-23: the conformance artifacts the protocol draft also puts
before the gate move to 2m, and Phase 2 closes after 2m.

- **Plaintext-canary test** ([plan](plan.md#testing-layers)). One end-to-end test
  drives everything Phase 2 built, putting a random canary in everything a
  writer writes: the pitch, notes in two channels, a reply, an edit, a
  deleted note, a channel's name and description, an attachment's name and
  contents. Along the way a second device is linked and gets the history by
  bundle, a backup is made and restored on a new device, which rejoins.
  - It then dumps **every table** in the database (found from the schema,
    so new tables are covered without changing the test) and searches the
    dump for the canary as text, hex, and base64 and base64url at each
    alignment.
  - It also searches it for every **secret** the clients hold: each
    persona's identity private key, each device's private keys (persona and
    account), each attachment's key, each blob token (the server keeps only
    hashes), and the recovery code (its words and its bits).
  - And it searches the **server's logs**, captured at the most detailed
    level for the whole run.
  - Positive controls show the search works: the prompt's public text is
    found as text, and a public key is found as hex.
  - The plan also names the federation queue, which arrives in Phase 5;
    the test gains it then.
- **Database audit** (`docs/server-data.md`): every column of every table,
  what it holds and why that's acceptable, in one of a few classes: public
  by design (the board, personas, public keys), what a login needs (login
  name, password hash, optional email), metadata (ids, times, sizes, log
  positions, who is in which session), ciphertext the server can't open,
  digests of secrets, and moderation text that reporters disclose. It ends
  with what the server can and can't learn. A test reads the schema and
  fails if a column is missing from the audit or listed but gone, so every
  migration must update it.
- **Uncertified devices rejected**, already proven: the server refuses
  certificates that don't verify (2b, `keys.rs`), clients refuse KeyPackages
  whose certificate doesn't verify against the pinned identity key (2c,
  `key_packages.rs`), and members refuse commits adding devices the
  directory doesn't list (2f, `sessions.rs`). The gate section of the audit
  names each test.

Done when: the canary test and the audit test pass in CI, the audit covers
every column, and the roadmap marks Phase 2's gate as passed pending 2m.

As built: the canary test checks 286 secrets in four dumps (after the story
is written, while a history bundle waits on the server, after a backup,
and at the end) and in the trace-level logs of the server and the
libraries it serves with (client libraries are left out: a client's logs
stay on its device). OpenMLS's secrets are found in its state by the
fields that name a private key or a secret. A negative control plants the
canary, base64url-encoded at an odd alignment, in a persona's public
profile and checks the search catches it.

### 2m: Conformance artifacts

The protocol draft (section 11) asks for three kinds of artifact before
Phase 2 ends, so that another implementation could be written from them:
an OpenAPI file for the client API, JSON Schemas for the objects inside
encrypted messages, and test vectors for every format. Each is checked in
CI against the code, so it can't drift.

- **OpenAPI file** (`docs/openapi.json`), generated from the code with
  `utoipa`.
  - `quill-proto`'s types derive their schemas behind an `openapi` feature
    that only the server turns on; clients don't link `utoipa`.
  - The server's API routers move to `utoipa-axum`, which registers each
    route together with its documentation, so a route can't be added
    without it. The board's HTML pages aren't part of the API and stay as
    they are.
  - It covers every `/api/v1` operation: parameters, bodies, responses and
    their error statuses (RFC 9457 problems, with the `409`, `410`, `413`
    and `429` cases), bearer auth, size limits (which the instance
    publishes), cursor paging, and the Server-Sent Events notice types.
  - Descriptions state the privacy behaviors: sign-in fails the same way
    whether or not the login exists, and a requester can't tell a held,
    pending, declined or expired request apart.
  - A test regenerates the file and fails if it differs from the one
    committed (`QUILL_WRITE_OPENAPI=1` rewrites it), so every API change
    shows up in review. An end-to-end test checks real responses from a
    running server against the file's schemas.
  - Fuzzing a server from the file (e.g. with Schemathesis) is part of 6b.
- **JSON Schemas** (`schemas/`, draft 2020-12), written by hand as the
  spec, for what OpenAPI doesn't cover: the activities in session messages
  (`Create`, `Update` and `Delete` of notes, `wr:Channel`), the
  `EncryptedAttachment`, the device group's payloads, and the rejoin
  request's envelope.
  - Tests validate what the client code actually produces against them,
    and check that must-reject examples fail both the schema and the
    client's parser, and that unknown terms are accepted by both.
  - The federated objects (`Prompt`, the session request's `Offer`, the
    actor extensions) don't exist as ActivityPub JSON until Phase 5; their
    schemas come with them, along with the `wr:` context document.
- **Test vectors** (`test-vectors/`), generated from fixed seeds by the
  crate holding each format, like 2b's, and checked both ways: regenerating
  them reproduces the committed files, and every case gives its recorded
  result through the public functions.
  - Linking: KeyPackage bytes to fingerprint, QR string and 6-word code;
    must reject a mismatched fingerprint, wrong words, a malformed code.
  - Backup: entropy to 12 words and passphrase and back; the plaintext's
    format; a fixed `age` file that opens with its code. Must reject a bad
    checksum, the wrong code and a damaged file. (`age` encryption is
    randomized, so the generator keeps the committed file as long as it
    still opens to the expected plaintext.)
  - Attachment: key, nonce and plaintext to padded blob and hash; must
    reject a changed tag, a wrong hash and a wrong blob size.
  - Ordering clock: formatting, parsing, the send and read rules, and
    display order including ties (clock, then sender, then `authorSeq`);
    must reject malformed values.
  - Tag and channel names: tricky Unicode to slug; must reject emoji, other
    scripts, empty names and over-length ones.
  - Padding and limits: blob size buckets, MLS padding, and each text limit
    exactly at and one past it; a server test sends a message exactly at
    the size limit and one byte over.
  - JSON objects: the schema examples above, as valid, unknown-terms and
    must-reject cases.
  - Not as vectors: the leaf extension (a fixed KeyPackage would expire;
    2c's `key_packages.rs` tests cover it), and MLS itself (OpenMLS runs
    the IETF vectors).
  - The draft also asks for checks on WebAssembly builds; those come with
    the web client.
- **The draft catches up** with what was built: the backup's plaintext is
  JSON, not CBOR (now zstd-compressed, as backup format version 2), so the
  "Pinned now" rule on deterministic CBOR goes,
  and section 11 links each artifact.

Done when: CI regenerates and checks the OpenAPI file, the schemas and
every vector file, the server's responses match the OpenAPI file, and the
roadmap marks Phase 2 done.

As built: the OpenAPI file covers 81 operations and validates as OpenAPI
3.1; operation ids come from the method and path, since handler names
repeat. The end-to-end check runs a story touching 71 of them and checks
about 200 JSON bodies; it found the string enums made by a macro
documented with Rust's variant names, which now write their schemas out.
A message exactly at the size limit passes the size check (and is refused
as malformed), one byte more gets 413. Vector files: `linking`, `backup`,
`attachment`, `ordering-clock` and `json-objects` from `quill-core`;
`names` and `limits` from `quill-proto`; `test-vectors/README.md` lists
them all. Schemas are checked with `boon`, a test-only dependency.

Reordered on 2026-10-23 (see Decisions): the desktop GUI and the web
client come next, before federation and hardening, so people can use the
project on one server without the CLI. Federation and hardening follow
unchanged in content.

## Phase 3: Desktop GUI

Epic: [#36](https://github.com/thecsapprentice/quill/issues/36). Each step is a sub-issue of it, with its scope and "done when"; this table stays the summary.

`quill-gui`, on Dioxus desktop ([plan](plan.md#future-clients-gui-framework-options)):
the app logic, `quill-core` included, runs as native Rust and only DOM
updates reach the webview, so keys never enter webview JavaScript. Views
live in a shared `quill-ui` crate the web client reuses. A first cut;
each sub-phase gets its detailed section when it's next.

Design principles (requested 2026-10-23), which every Phase 3 sub-phase
is reviewed against, and the web client inherits:

- **Friendly first.** The CLI may be technical: ids, explicit key set-up,
  manual steps. The GUI's job is to let people post a prompt and start
  writing with someone as fluidly as possible. Without weakening any
  protection, it removes every step a person doesn't need to decide:
  - Keys are made and published on first run, KeyPackages are kept topped
    up, history syncs and sessions are rejoined in the background, and
    nobody sees an id, a key or a log position.
  - Linking a device is scanning a QR code (or typing six words); a
    backup is offered, with its recovery code, at a moment that doesn't
    interrupt writing, and reminded about until one exists.
  - Security that needs a person stays, made plain and quick: comparing
    safety numbers is offered, never forced; a changed identity key or a
    linking code that doesn't match always stops and explains, in words,
    what happened and what to do.
  - An **advanced mode** may show what the CLI shows (ids, device keys,
    log positions, KeyPackage pools, raw errors) for those who want it;
    nothing ordinary needs it.
- **Responsive and alive.** Layouts adapt to the window, from a narrow
  side-by-side window to full screen (lists and the session collapse into
  one column when narrow). The interface is interactive throughout:
  immediate feedback on every action, live updates as messages arrive,
  drag and drop for files, keyboard shortcuts, and clear progress for
  anything slow, so nothing makes people wait without knowing why.
- **A pleasant place to write.** The editor gets out of the way:
  - Spellcheck, and input methods for every language, through the
    platform's own text input.
  - Markdown with a live preview (formatting shown as you type) and a
    side-by-side or toggled preview; plain shortcuts for the common
    marks.
  - Long-form writing that stays smooth in long sessions; drafts saved
    as you type (in the encrypted store) and never lost to a closed
    window or a dropped connection; sending that queues while offline.
  - A focus mode without the surrounding chrome.
  - No modal dialogs or confirmations in the way of writing; mistakes
    are undone (an edit's history is kept anyway) rather than prevented
    by asking.

3a's spike tests these on both targets before Dioxus is confirmed: the
editor's spellcheck, input methods, live preview and responsiveness are
part of its "done when".

| Sub-phase | Scope | Status |
| --- | --- | --- |
| 3a | Spike: the session view (channels, the long-form editor with Markdown live preview, an image attachment) in Dioxus on desktop and in the browser, at narrow and wide window sizes; IME, spellcheck and screen readers; confirm Dioxus or fall back to Leptos + Tauri 2 | done (#23) |
| 3b | [#25](https://github.com/thecsapprentice/quill/issues/25) `quill-ui` and the `quill-gui` shell: first run (sign up or in, a persona, keys made and published without being asked), unlocking the store, personas, settings, advanced mode, the server rules | in review (#63) |
| 3c | [#26](https://github.com/thecsapprentice/quill/issues/26) The board in the GUI: browse, search, post and edit prompts, through the board's shared components | planned |
| 3d | [#27](https://github.com/thecsapprentice/quill/issues/27) Keys and devices: identity keys, linking by QR code or words, the device list, revoking, safety numbers | planned |
| 3e | [#28](https://github.com/thecsapprentice/quill/issues/28) Requests: sending with a pitch, the inbox, accepting and declining, status | planned |
| 3f | [#29](https://github.com/thecsapprentice/quill/issues/29) Sessions: the session list, reading and writing, channels, replies, edits and deletions with their history, live updates | planned |
| 3g | [#30](https://github.com/thecsapprentice/quill/issues/30) Attachments: attaching, opening and saving files; images shown inline, with metadata stripped before sending (deferred from 2i); storage use | planned |
| 3h | [#31](https://github.com/thecsapprentice/quill/issues/31) Backup and recovery, history sync and rejoining, notices; anything the CLI does that the GUI doesn't yet | planned |
| 3i | [#32](https://github.com/thecsapprentice/quill/issues/32) Packaging for Windows, macOS and Linux | planned |

Gate: the GUI does everything the CLI does, and the end-to-end tests that
drive `quill-core` pass unchanged. A new user can go from installing to
writing in a session without seeing an id or a key, and without leaving
the app.

### 3a: Spike: the session view and the editor

The plan asks for a short spike before the GUI is built on Dioxus: the
riskiest view, built quickly on both targets, to find out whether Dioxus
(0.7, already used for the board) can give writers the editor Phase 3's
principles ask for. It is throwaway code that answers questions; what it
learns decides 3b.

- **Where it lives.** `spikes/gui/`, its own small Cargo workspace outside
  the main one, so its dependencies (the desktop webview stack) don't enter
  the project's lockfile, CI or `cargo vet` until 3b decides what to
  adopt. One crate builds both targets: `--features desktop` (Dioxus
  desktop: WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux)
  and `--features web` (WebAssembly in the browser, through `dx serve`).
- **What it shows.** One session, with made-up notes, no networking or
  keys:
  - The channel list, the notes in the selected channel (rendered Markdown,
    sanitized as the real client must), and replying to one.
  - The editor, in the three forms worth comparing:
    1. a plain text area with a preview beside it or toggled;
    2. live preview in place: the Markdown source styled as you type
       (headings, emphasis, quotes, code), drawn behind a transparent text
       area so typing, spellcheck and input methods stay the platform's
       own;
    3. a rich editing surface (`contenteditable`), formatted in place.
  - Shortcuts for bold, italic and sending; a draft kept as you type and
    restored when switching channels.
  - Attaching an image by picking or dropping it, shown inline.
  - A layout that goes from one column in a narrow window to channels,
    notes and editor side by side when wide.
- **What it tests**, on Windows, macOS and Linux desktops and in Chrome,
  Firefox and Safari, recorded in `spikes/gui/FINDINGS.md`:
  - Spellcheck (underlines and suggestions) in each editor form.
  - Input methods: composing Japanese or Chinese, dead keys and accents,
    emoji pickers, without lost or doubled characters.
  - Screen readers (Narrator or NVDA, VoiceOver, Orca): the notes and the
    editor are announced and usable from the keyboard.
  - Responsiveness: resizing between narrow and wide; typing stays smooth
    in a note of 20,000 words.
  - Building: the desktop binary on each platform, the web build's size.
- **Deciding.** The findings pick the editor form for 3f, and confirm
  Dioxus or move to Leptos + Tauri 2 (the plan's fallback) before 3b. The
  checks that need a person at each machine (input methods, screen
  readers, the look on each platform) are done by hand with the spike
  running; the findings say who checked what.

Done when: the spike builds and runs on both targets; `FINDINGS.md`
covers every check above on at least Windows and one browser, with the
rest marked as still to check; and the roadmap records the decision.

As built (`spikes/gui/FINDINGS.md` has the details):

- **Dioxus is confirmed** (0.7.10). The spike builds unchanged for
  Windows, macOS, Linux and the browser; the desktop app is 4.9 to 6.6 MB
  with the system's webview, the WebAssembly module 0.6 MB gzipped.
  Checked by hand on the Windows desktop (as expected) and in Edge;
  scripted on Linux.
- **The editor for 3f is Live**: the Markdown styled in place behind a
  transparent text area, so typing, selection, spellcheck and input
  methods stay the platform's own. Styles must not change character
  widths, so while typing bold is a faint shadow, emphasis a colour, and
  headings aren't larger. **Write + preview** is the fallback, and for very
  long notes. **Rich** (`contenteditable`) is dropped: it rewrites the
  writer's Markdown.
- **Rules for every text input in the GUI:**
  - Uncontrolled: a controlled text area lost about a third of fast
    typing on the desktop. Rust replaces the text only by remounting.
  - Remount through a one-item keyed list: Dioxus ignores `key` on a lone
    element.
  - `writingsuggestions="false"` and `autocomplete="off"`: Edge's text
    prediction appeared in the browser, and a browser's writing assistant
    may send text off the device.
  - On Linux, spellcheck is switched on through WebKitGTK.
- **For 3f:** re-render the preview a few times a second, not per
  keystroke; measure Live on real hardware with long notes (it trailed at
  the end of 20,000 words under software rendering, without losing
  anything). Edge showed preview lists without markers; styles now set
  them outright, to confirm.
- **Still to check by hand, carried into the sub-phases that build each
  part:** input methods and screen readers (3b onward), macOS, Linux
  desktops, Chrome, Firefox and Safari (3f for the editor; Phase 4 for the
  browsers).
- `spikes/gui/` stays as a reference until 3f's editor replaces it, then
  goes; its workflow runs only when it changes.

## Phase 4: Web client

`quill-web`: Rust compiled to WebAssembly, on its own origin, sharing
`quill-core` and `quill-ui` with the desktop GUI. It must adopt every
mitigation in the plan's
[web client security requirements](plan.md#web-client-security-requirements-for-the-separate-proposal).
A first cut.

| Sub-phase | Scope | Status |
| --- | --- | --- |
| 4a | The web client proposal: how each security requirement is met, the threat model's web-specific risks, and what the server serves | planned |
| 4b | `quill-core` on WebAssembly: the store over IndexedDB wrapped by a non-extractable WebCrypto key, HTTP through `fetch`, the RustCrypto provider, the browser's randomness; the test vectors run on WebAssembly in CI (deferred from 2m) | planned |
| 4c | `quill-web` on its own origin: the shared views, strict CSP with Trusted Types, decrypted content in a sandboxed frame | planned |
| 4d | Web devices: linking as a limited device by default, onboarding to a backup or a second device, persistent storage, warnings about clearing site data | planned |
| 4e | Verifiable code: reproducible builds, a signed release manifest, Subresource Integrity, a pinned version through a service worker, loading the bundle from any origin | planned |

Gate: the web client passes the same end-to-end tests in a browser,
including the plaintext-canary test, and a second build from the same
source is byte-identical.

## Phase 5: Federation

| Sub-phase | Scope |
| --- | --- |
| 5a | Actors, WebFinger, HTTP Signatures |
| 5b | Board federation: follows, prompt delivery, remote prompts on the board |
| 5c | Remote collaboration requests and KeyPackage claims |
| 5d | Cross-server sessions sequenced by the prompt author's server |
| 5e | Two-instance test harness; interop with a real Mastodon instance |

The GUI and web client gain what federation adds (remote prompts and
writers) in the same sub-phases as the CLI.

## Phase 6: Hardening and review

| Sub-phase | Scope |
| --- | --- |
| 6a | Malicious-server test suite, run against every client |
| 6b | Fuzzing of every untrusted-input parser, and of the API from the OpenAPI file |
| 6c | Operations runbook, disclosure policy, Tor onion service |
| 6d | Settings review: every hard-coded limit and tuning value becomes a server setting or a client configuration entry (see below) |
| 6e | External review and fixes, the web client included |

### 6d: Settings review (planned)

Requested 2026-10-23. Many values are constants in the code today; this
pass lists them all and moves each into server settings or a client
configuration file, with today's values as defaults and documented safe
ranges. Known so far:

- **Client:** the KeyPackage pool target (20) and refill threshold (10),
  last-resort renewal window (7 days), KeyPackage lifetime (28 days),
  passphrase minimum (12 characters) and Argon2id cost.
- **Server:**
  - Limits: KeyPackages per device (100), claims per account per hour
    (60), link mailbox lifetime (10 minutes) and links per hour (10),
    reports per hour.
  - Admin log: page sizes, accounts shown per row (5).
  - Certificates: clock-skew allowance (10 minutes).
  - Requests and sessions: request expiry (30 days), requests per hour
    (20), message retention grace period (7 days), low-pool notice
    threshold (10), live notice buffer (1024).
  - Retention (2k): the sweep's interval (1 hour). The retention period
    itself is `QUILL_RETENTION_DAYS` already.
  - Attachments: size limit (10 MiB), smallest bucket (4 KiB), quota per
    login (500 MiB), blobs per message (10), unattached-blob lifetime (1
    day), messages per minute (30). These are in the config's limits but
    not yet read from the environment.
- **Client (sessions):** message padding (256 bytes), how far ahead a
  clock may be and still move the reader's (1 day), longest pitch and
  message.
- **Client (history, 2k):** asking again for history (7 days), dropping
  unanswered requests (30 days), waiting for a sibling before asking the
  other writer to rejoin (1 day), history part size (4 MiB).

## Decisions

- **#43, automated moderation (decided 2026-10-26): Rhai, with default
  templates and common data.** The spike ran the same scripts in rule files,
  Rhai, Luau, Starlark and JavaScript; Rhai stopped every hostile script with
  built-in limits and is pure Rust. Templates, not a second simpler
  language, serve admins who don't write code. Common data is admin-kept
  named strings that scripts read but can't change, parsed on the server
  once per version. A failing script's effect is set per chain; the first
  release ships `reject` and `flag` (`hold` is #44).
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
- **2a and 2b, key scope (decided 2026-10-22): one identity key per persona.**
  A shared per-user key, as first drafted, would publicly link every persona
  of a login. Each device keeps a device key and certificate per persona.
- **2a, order (decided 2026-10-22): the encrypted store comes first**, so no
  private key is ever written to disk unencrypted, even temporarily.
- **2a, store backend (decided 2026-10-22): redb with our own page
  encryption, not SQLCipher.** SQLCipher needs OpenSSL's libcrypto (system or
  vendored C) on every client platform and can't run in the browser, where
  the web client will need its own encryption anyway. Turso's page
  encryption is still experimental; the redb fork with built-in encryption
  has one maintainer and little use. The trade: the page-encryption layer is
  project code and goes into the external review, and there's no SQL. To
  keep the choice cheap to revisit, all storage goes through a four-method
  interface with a shared conformance test. Swapping to SQLCipher means one
  new backend module (about 150 lines, since the interface is plain tables of
  keys and values), dropping the page layer, and adding OpenSSL to client
  builds; nothing else changes. Once real users exist, a one-time local
  conversion is needed too. Decide for good before the Phase 6 review.
- **2e, scope (decided 2026-10-23):** the request carries the session's
  real MLS group from the start, as the protocol draft says, so 2e builds
  the group, the Welcome and the pitch, and acceptance registers the group
  on the message log. 2f covers sessions from there: the session list,
  Welcomes for members' other and later devices, writing, and Leave.
- **2j, backups and scope (decided 2026-10-23):** backups use the `age`
  format through the `age` crate (about 100 more client crates), behind an
  interface so our own implementation of the format can replace the crate
  without other changes. History sync for later devices moves to 2k; the
  phase gate becomes 2l.
- **2l, gate and conformance (decided 2026-10-23):** 2l builds the
  plaintext-canary test and the database audit; the OpenAPI file, JSON
  Schemas and remaining test vectors become 2m, and Phase 2 closes after
  it, before federation needs them.
- **GUI design principles (decided 2026-10-23):** friendly first, with an
  advanced mode for what the CLI shows; responsive to the window and
  interactive throughout; a writing environment with spellcheck and
  Markdown live preview that gets out of the way. Listed under Phase 3.
- **Phase order (decided 2026-10-23):** the desktop GUI (now Phase 3)
  and the web client (Phase 4) move ahead of federation (Phase 5) and
  hardening and review (Phase 6), so the project is usable on one server
  without the CLI sooner. Sub-phases keep their content under the new
  numbers (the settings review is 6d). Hardening still comes before any
  public launch, and it now covers the web client too.
- **2m, conformance (decided 2026-10-23):** the backup's plaintext stays
  JSON as built (one fewer format in the client; nothing signs or hashes
  it, so it needs no deterministic encoding) but is now zstd-compressed,
  as the draft had it, to win back the size base64 costs; the draft
  changes to match. JSON Schemas are written by hand, since they are the spec, and
  checked against the code; the OpenAPI file is generated from it.
- **2i, attachments (decided 2026-10-23):** any file type, size caps only;
  no type checks, which renaming defeats. Image cleaning (metadata
  stripping, re-encoding) and rendering wait for the GUI. Administrators
  can turn attachments off, and logins and administrators can see storage
  use (sizes only).
- **2d, scope (decided 2026-10-23):** the group message log and epoch
  sequencer move from 2f into 2d, since the personal device group needs
  them; 2f builds session groups on them. Only full devices are linked;
  limited devices come with the web client.
