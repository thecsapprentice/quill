# Rebranding Quill as Plait

Status: proposed. Decided 2026-11-02: the project is renamed **Plait**,
and its vocabulary comes from weaving. This document records what the
rename covers, which words change where, and how to carry it out. The
brand assets and their usage rules are in
[`assets/brand/README.md`](../assets/brand/README.md).

## Why Plait, and why weaving

"Quill" is taken in our own space: an IndieWeb publishing client
(Micropub), the widely used Quill.js rich-text editor (which owns the
`quill` npm package), Ghost's Android app, QuillBot and Quill.org. A
collaborative writing platform named Quill would be confused with at least
two of them. "Plait" has no writing or fediverse software using it. Its
nearest uses are a restaurant scheduling app, a medical-practice inbox and
a dormant JavaScript framework.

Plait is for writing *together*, which a plait shows: separate strands
that hold only because they cross. One family of words, weaving, covers
both halves of the product: the public board where ideas are strung up,
and the private looms where writers weave through them. The same family
gives the art something to draw.

## Vocabulary

Every theme word has one of three reaches:

- **Everywhere.** The brand: code, crates, binaries, files, docs and UI.
- **Surface.** Labels, copy, docs and art. Code, the API, the CLI's
  subcommands, the database and the `wr:` protocol keep the plain word.
- **Art only.** Icons, illustrations and marketing. The UI label stays
  plain, because the theme word would cost clarity.

| Today | Theme word | Reach | In the UI | Code, API, protocol |
| --- | --- | --- | --- | --- |
| Quill | **Plait** | Everywhere | Plait | `plait`, `plait-*`, `PLAIT_*` |
| Prompt | **Warp** | Surface | "Warp", "Post a warp", "My warps" | `Prompt`, `/prompts`, `wr:Prompt` |
| Prompt board | Board (of warps) | Surface | "Board" (unchanged), warp-board icon | unchanged |
| Writing session | **Loom** | Surface | "Looms", "Open the loom", "locked loom" for the encryption badge | `Session`, `/sessions`, MLS group |
| Session request | Shuttle | Art only, plus a verb | Noun stays "Request"; the button becomes **"Ask to weave"**; shuttle icons | `Request`, `/requests` |
| A contribution | Strand | Art only (surface later, if the writing screen wants a noun) | | AS2 `Note` / `Article` |
| Story and planning channels | Cloth and draft | Art only | "Story", "Planning"; cloth and draft icons | `wr:channel` values `story`, `planning` |
| Finished, exported piece | Bolt | Art only | "Export"; bolt icon | |
| Persona | Thread | Art only | "Persona" | `Persona` |
| Server | Guild | Art only | "Server" | unchanged |
| Safety number, verified | Selvedge | Art only | "Safety number" | unchanged |
| Edit | Needle | Art only | "Edit"; needle icon | |
| Tags, notices, news, reports, rules, moderation | none | | unchanged | unchanged |

Why some theme words stay off the labels:

- **Persona is not "thread".** Threads is Meta's network, and it federates
  over ActivityPub. "Your threads" in a fediverse app would be read as
  posts on Threads. The spool icon carries the idea without the clash.
- **Server is not "guild".** Fediverse users already say server or
  instance, and one of our principles is a low barrier for people coming
  from established apps (CLAUDE.md). Guild suits marketing, not settings.
- **Channels keep their names.** `story` and `planning` are values inside
  encrypted payloads (protocol draft, `wr:channel`). Renaming what the UI
  shows is free; renaming the values would be a protocol change for
  nothing.
- **Code keeps the plain domain words.** The protocol draft, the OpenAPI
  file, the JSON Schemas and the test vectors are written for other
  implementers. "Prompt", "session" and "request" are self-explanatory
  there. "Warp" and "loom" are presentation. Renaming the domain model
  would touch most of the codebase and every conformance artifact, and
  gain nothing a reader of the code needs.

The two surface words need one line of explanation where people first
meet them: on Home and in the first-run copy. For example: "A **warp** is
a prompt: the idea others weave through. A **loom** is a private,
encrypted space where you write with someone." The board's empty state
and the Looms page's empty state repeat it.

## How the rename lands in each part of the app

**Desktop app (`plait-ui`, `plait-gui`).**
- Window title, the root element's class (`.quill` → `.plait`) and the nav's
  `aria_label` change with the brand.
- Nav labels live in `Section::label()` (`model.rs`): "My prompts" →
  "My warps", "Sessions" → "Looms". The `Section` variants keep their
  names.
- First run (`first_run.rs`): the hero, a plait band on the card, and the
  shuttle loader on `Starting` and `PreparingStep`. Copy that says
  "Quill" changes with the brand ("Plait is making the keys…").
- Requests page: "Ask to weave" for the action, shuttle-sent and
  shuttle-received icons for the two lists.
- Looms page and the writing space: loom icons; the locked loom marks
  end-to-end encryption; cloth and draft icons on the channel tabs; needle
  for edit; bolt for export.
- Personas keep their name and get the spool icon.

**Public board (`plait-board-ui`, `plait-server/src/web`).**
- Header and page titles: the lockup, and "warp" in place of "prompt"
  wherever writers read it: "Post a warp", "My warps", the warp page.
  Admin pages keep the plain words, because administrators work across the
  API and the CLI as well.
- The nav icons in `layout.rs` are replaced by the shared icon module
  (below).
- A favicon, an Apple touch icon, and `og:` tags with the social card.

**CLI (`plait`).** The binary is renamed. Subcommands mirror the API and
keep their names (`plait prompt post`, `plait session …`). If wanted, add
`visible_alias = "warp"` and `"loom"` in clap so either word works.

**Server and API.** Routes, JSON fields, the database, `wr:` terms and the
MLS signature labels (`wr device cert v1` and the rest) are unchanged.
Only branded identifiers change: the nodeinfo `software` name, the client
user agent, cookie names and environment variables.

**Data on disk and on the wire.** These are format identifiers, and
changing them makes old local data unreadable. That is acceptable before
the first release, and will never be again, which is why the rename should
happen now:

| Identifier | Where | Becomes |
| --- | --- | --- |
| Store magic `QUILLSDB` | `store/header.rs` | `PLAITSDB` (same 8 bytes) |
| Page label `quill store pages v1` | `store/mod.rs` | `plait store pages v1` |
| Store file `store.quill`, `{id}.quill` | `store/mod.rs`, `native.rs` | `.plait` |
| Config and data folders `quill`, `quill-gui` | `ProjectDirs` in `profile.rs`, `store/mod.rs`, `settings.rs` | `plait`, `plait-gui` |
| Keychain service `quill-store` | `store/vault.rs` | `plait-store` |
| Backup format `quill-backup` | `backup/mod.rs` | `plait-backup` |
| Linking codes `quill-link:1:`, `quill-device:` | `devices.rs` | `plait-link:1:`, `plait-device:` |
| Cookies `__Host-quill_session`, `__Host-quill_dismissed` | `web/session.rs`, `web/announcements.rs` | `__Host-plait_…` (everyone is signed out once) |
| User agent `quill/…`, nodeinfo `software: "quill"` | `client.rs`, `routes.rs`, `api.rs` | `plait` |
| Namespace placeholder `quill.example/ns/wr` | `activity.rs`, `schemas/`, protocol draft | `plait.example` until the project domain is chosen |

## Where the assets live

- `assets/brand/`: the generated art and its generators, the source of
  truth (see its README).
- `crates/plait-board-ui/src/icons.rs`: the shared `Icon` enum, copied
  from `assets/brand/icons/icons.rs`. The desktop UI already depends on
  the board UI, so one copy serves both. The server links nothing new:
  only strings, nothing near the dependency ban.
- `crates/plait-board-ui/src/art.rs`: `include_str!` constants for the
  hero, band, loader and illustrations, used by both UIs.
- `plait-gui`: the window icon (`include_bytes!` of
  `assets/brand/app-icon/png/plait-256.png`). The installer icons are
  referenced from `assets/brand/app-icon/` by the bundler's config.
- `plait-server`: the favicon, touch icon and social card, served with
  `include_bytes!` from `assets/brand/`. The Dockerfile copies the whole
  repository, so nothing else is needed.

## Carrying it out

Work goes in as GitHub issues under one epic (CLAUDE.md), each a small
PR. The order matters: mechanical renames first, while nothing else is in
flight; vocabulary last, because it changes the labels the interaction
tests select by.

Most of the brand rename is mechanical and done by
[`scripts/rebrand-to-plait.sh`](../scripts/rebrand-to-plait.sh), in two
phases with a commit after each, plus an audit:

```sh
git switch -c rebrand/crates
scripts/rebrand-to-plait.sh crates   # git mv crates/quill* → crates/plait*, package names, Rust paths
git commit -am "Rename crates to plait-*"
scripts/rebrand-to-plait.sh brand    # every other quill/Quill/QUILL outside the keep list
scripts/rebrand-to-plait.sh audit    # what's left, which should be only the keep list
```

Tried on `main` at 6c38e3f: the two phases change 268 files. `cargo
metadata` resolves all eleven renamed packages. The audit then lists only
the keep list below plus `test-vectors/*.json`, which is regenerated.
Nothing was compiled, because the pinned toolchain wasn't available where
this was tried, so CI is the check.

### The keep list (left saying "quill" on purpose)

- `spikes/` and `docs/design/`: dated records of past work.
- Migrations `0001`–`0029`. sqlx checksums applied migrations, so even a
  comment edit breaks every existing database. New names go in a new
  migration.
- `test-vectors/*.json`: regenerated with `PLAIT_WRITE_VECTORS=1`, never
  edited. Some vectors depend on the labels.
- `words.rs`'s test, where "quill" stands for a word that isn't in the
  BIP-39 list.
- The reserved-handle defaults keep **quill** alongside the new **plait**,
  so nobody can pass for the old name either.

### Issues to open

**Epic: Rebrand to Plait** (labelled `epic`), with these as sub-issues:

**R1: Brand assets and this guide.** Scope: `assets/brand/`, this
document, the rename script. Done when: merged; the asset README and this
guide reviewed.

**R2: Rename crates and binaries.** Scope: `rebrand-to-plait.sh crates`:
directories, package and binary names (`plait`, `plait-server`,
`plait-gui`, `plait-seed`), Rust paths, `Cargo.lock`, workspace
dependencies, the Dockerfile, CI, `scripts/check-server-deps.sh`. Done
when: CI is green; `scripts/check-server-deps.sh` passes **and** fails if
`plait-core` is added to `plait-server`'s dependencies. Try that on a
scratch commit: the ban list matches crate names, so a stale name would
pass silently. The Docker image also needs to build.

**R3: Rename everything else that says Quill.** Scope:
`rebrand-to-plait.sh brand`: environment variables (`QUILL_*` →
`PLAIT_*`, no fallback before the first release), the format identifiers
in the table above, cookies, the user agent, nodeinfo, prose in docs and
error messages, `compose.yaml` and `scripts/dev-gui.py`. Then, by hand:
regenerate the test vectors and the OpenAPI file and review their diffs;
add `"quill"` back to `DEFAULT_WORDS` and a `the_quill` case to the
handles test; add migration `0030_reserved_plait.sql` (`INSERT INTO
reserved_handles (pattern, kind, source) VALUES ('plait', 'word',
'default') ON CONFLICT DO NOTHING`), because defaults are seeded only once
per database. Done when: CI is green; `audit` shows only the keep list; a
fresh `docker compose up` plus the seed works; `dev-gui.py` opens the
GUI.

**R4: Desktop app identity.** Scope: window icon (`Config::with_icon`,
after `with_window`); the `--thread-*` and `--wood` tokens in `style.css`;
installer icons wired to the bundler. Done when: the icon shows in the
title bar and taskbar on Windows, macOS and Linux (hand check), and the
tokens exist in all three theme blocks.

**R5: Welcome screens.** Scope: the hero and plait band in
`first_run.rs`'s `Card`, the shuttle loader on `Starting` and
`PreparingStep`, the CSS from the asset README. Done when: the existing
first-run interaction tests pass unchanged (the art is `aria-hidden`); a
view test checks the hero is present and hidden from assistive tech; hand
checks at 360px and 1180px wide, in light, dark and system themes, with
reduced motion on.

**R6: Shared icons and empty states.** Scope: `icons.rs` and `art.rs` in
`plait-board-ui`; the board's `layout.rs` nav uses them; the desktop nav
gets an icon per `Section`; the empty board, requests and looms
illustrations; the offline illustration in the status area's "can't
reach" state. Done when: every nav entry has an icon with its text label;
icon-only buttons have an `aria-label`; interaction tests still pass.

**R7: Public board identity.** Scope: `/favicon.ico`, `/favicon.svg` and
`/apple-touch-icon.png` routes (and the e2e check in `board.rs` that
expects 204 now); `link rel="icon"` and `og:` tags in the board's `head`;
the lockup in the header; the hero on the sign-in page. Done when: the
e2e tests cover the three routes and the tags.

**R8: Vocabulary.** Scope: a `vocab` module in `plait-board-ui` holding
the user-facing nouns and verbs ("warp", "loom", "Ask to weave"), used by
both UIs, so the next change is one file; `Section::label()`; board and
desktop copy; the explanations on Home, first run and empty states;
optional CLI aliases; a glossary section in `docs/plan.md`. Done when:
interaction tests select by the `vocab` constants, not by literals; the
GUI flows in `plait-e2e` pass; no user-facing "prompt" or "session" is
left outside admin pages (grep the UI crates).

**R9: Repository and infrastructure.** Scope: rename the GitHub
repository to `plait` (GitHub redirects the old URLs) *before* R3 merges,
since R3 rewrites the repository URL and the CI badge. Update local
remotes. Rename the compose project, images and Postgres role. Old local
databases live in volumes under the old project name: remove them with
`docker compose -p quill down -v`. Choose the project domain, then replace
the `plait.example` placeholder. Done when: links in the README, CI and
`Cargo.toml` resolve.

### For developers, once R3 lands

Local data from before the rename isn't read any more: the store's file
name, folder, magic bytes and keychain entry all changed. Delete
`quill`/`quill-gui` from your config and data folders, the `quill-store`
keychain entries, and the old Docker volumes, and start a fresh profile.

## Risks

- **The server dependency ban.** It matches names, so it must be renamed
  in the same commit as the crates, and shown to still fail (R2).
- **Applied migrations.** Never touched (keep list).
- **Labels in tests.** R8 changes what tests click. Doing it last, and
  through `vocab` constants, keeps that to one PR.
- **Name clashes for "plait".** No writing or fediverse software uses it.
  An old Ubuntu package installed a `plait` command (a jukebox), so the
  Linux packages should check for a conflicting `/usr/bin/plait`. Domains
  and handles for the bare word may be taken. A qualified form
  (`plait.ink`, `plait.social`, `@plait@…`) is the fallback, and should
  be checked before R9.
