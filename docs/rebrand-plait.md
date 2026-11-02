# Rebranding Quill as Plait

Status: proposed. Decided 2026-11-02: the project is renamed **Plait**,
and its art and voice come from weaving. The interface keeps writing
words. This document records what the
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
that hold only because they cross. Weaving gives the brand a coherent
world to draw from: a public board where ideas are strung up like warp
threads, and private sessions where writers weave through them. That
world belongs in the logo, the art and the docs. People using the app
still read writing words.

## Vocabulary: writing words on screen, weaving words around them

People arrive to write, not to learn a craft. Most of them don't know
what a warp or a selvedge is, and a new word on every button makes an
unfamiliar app feel harder than it is. Our own principle asks for
"smooth, low-barrier clients" (CLAUDE.md). So:

- **Every label, button, heading, setting and error uses the writing
  word**: prompt, session, request, persona, server. These are unchanged
  from today.
- **Only the brand name changes everywhere**: Quill becomes Plait.
- **The weaving words are a layer of meaning around the interface**,
  never a replacement for it. They appear where people can enjoy them
  but never need them to find their way.

| Writing word (UI, code, API) | Weaving counterpart | Where the counterpart appears |
| --- | --- | --- |
| Prompt | Warp | Art, docs, tagline, one onboarding line |
| Prompt board | (the warp strung on the loom) | Hero and empty-board art |
| Writing session | Loom | Art, docs, the session icon |
| Request to write together | Shuttle | Art; the request icons |
| A contribution | Strand | Art and docs |
| Story and planning channels | Cloth and draft | Icons only |
| Exported piece | Bolt | Icon only |
| Persona | Thread | Art only: each writer is a thread of their own colour |
| Server | Guild | Not used: fediverse users say server or instance |
| Safety number | Selvedge | Not used: security wording must be plain |

### Where the weaving words do appear

From most to least visible:

1. **The brand.** The name, the plait mark and the hero art. The tagline
   is plain: "Prompts in public. Writing together, in private."
2. **The art.** Illustrations and icons draw the metaphor: the board's
   empty state is warp threads waiting, the requests empty state is a
   shuttle at rest, the session icon is a loom. Their accessible names
   are the writing words ("A writing session"), so screen-reader users
   never meet the metaphor unexplained.
3. **One line of onboarding**, optional and skippable, on the first-run
   card or Home: "Why Plait? Writing together is like weaving: a prompt
   is the warp, strung up for others to weave through." It explains the
   name. Nothing later depends on remembering it.
4. **The docs and the website**, where there's room to tell the story.
   Lead with the writing word and give the weaving word as the aside:
   "**Prompts** (the warp)", "**Sessions** (looms)". Never the other
   way round, and never a weaving word alone in a heading.
5. **Light flavour in non-functional copy**, sparingly: an empty state's
   second line ("Nothing on the loom yet"), a release-notes headline.
   The first line of any empty state still says plainly what's missing
   and what to do ("No sessions yet. Find a prompt on the board and ask
   to write it together.").

Things to avoid:

- **Hybrid labels such as "Warps (Prompts)".** They double the reading
  load in the busiest places (navigation, buttons, page titles), and they
  put the unfamiliar word first. If a hybrid is wanted anywhere, make it
  "Prompts (warps)", and only in docs.
- **Weaving words in errors, security and moderation text.** "This
  session's safety number changed" must stay as plain as it is today.
- **Renaming code to match the art.** The domain model, routes, CLI
  subcommands, `wr:` terms and icon names all use the writing words, so
  the code reads the same as the screen. The icon module follows this:
  `Icon::Session` is drawn as a loom.

If, after launch, people start using "warp" and "loom" on their own, the
words can be promoted to the interface then. That's easy to add later,
and hard to take back once it confuses people.

## How the rename lands in each part of the app

**Desktop app (`plait-ui`, `plait-gui`).**
- Window title, the root element's class (`.quill` → `.plait`) and the
  nav's `aria_label` change with the brand. `Section::label()` is
  unchanged: Board, My prompts, Requests, Sessions, Personas.
- First run (`first_run.rs`): the hero, a plait band on the card, and the
  shuttle loader on `Starting` and `PreparingStep`. Copy that says "Quill"
  changes with the brand ("Welcome to Plait", "Plait is making the keys…").
  Optionally, one "Why Plait?" line (see Vocabulary).
- Icons beside every nav entry and on the Requests and Sessions pages,
  from the shared module. Each keeps its text label.
- Empty states use the illustrations, with plain first lines.

**Public board (`plait-board-ui`, `plait-server/src/web`).**
- Header and page titles: the lockup. Labels are unchanged.
- The nav icons in `layout.rs` are replaced by the shared icon module
  (below), with the same paths for the icons it already has.
- A favicon, an Apple touch icon, and `og:` tags with the social card.
- The sign-in page can carry the hero like the desktop app.

**CLI (`plait`).** The binary is renamed. Subcommands mirror the API and
keep their names (`plait prompt post`, `plait session …`).

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
PR. Mechanical renames come first, while nothing else is in flight. The
interface keeps its labels, so the interaction tests keep selecting by the
same words throughout.

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
gets an icon per `Section`; the empty board, requests and sessions
illustrations; the offline illustration in the status area's "can't
reach" state. Done when: every nav entry has an icon with its text label;
icon-only buttons have an `aria-label`; interaction tests still pass.

**R7: Public board identity.** Scope: `/favicon.ico`, `/favicon.svg` and
`/apple-touch-icon.png` routes (and the e2e check in `board.rs` that
expects 204 now); `link rel="icon"` and `og:` tags in the board's `head`;
the lockup in the header; the hero on the sign-in page. Done when: the
e2e tests cover the three routes and the tags.

**R8: Copy pass.** Scope: every user-facing "Quill" in both UIs and the
docs becomes "Plait" (most of this lands in R3). Rewrite empty states
with a plain first line and an optional flavour line. Add the optional
"Why Plait?" line to first run or Home. Add a short "About the name"
section to the README and the docs, using "Prompts (the warp)" style
asides. No label, button or heading changes its writing word. Done when:
the interaction tests and the GUI flows in `plait-e2e` pass unchanged;
`grep -ri "warp\|loom" crates/plait-ui crates/plait-board-ui` finds only
icon art, flavour lines and comments.

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
- **Comprehension.** The weaving theme is unfamiliar to most people. It
  stays out of labels and functional text (see Vocabulary), so nobody has
  to learn it to use the app.
- **Name clashes for "plait".** No writing or fediverse software uses it.
  An old Ubuntu package installed a `plait` command (a jukebox), so the
  Linux packages should check for a conflicting `/usr/bin/plait`. Domains
  and handles for the bare word may be taken. A qualified form
  (`plait.ink`, `plait.social`, `@plait@…`) is the fallback, and should
  be checked before R9.
