[![CI](https://github.com/thecsapprentice/quill/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/thecsapprentice/quill/actions/workflows/ci.yml)

# Quill

A federated, pseudonymous collaborative-writing server. Writers post public
**prompts** to a tag-searchable bulletin board and open end-to-end encrypted
**writing sessions** with each other. The server relays and stores session
messages but can never read them: sessions use
[Messaging Layer Security (RFC 9420)](https://www.rfc-editor.org/rfc/rfc9420.html),
and servers federate over ActivityPub.

> **Status: Phase 1 (prompt board) complete.** Accounts, personas, prompts,
> tags, search, reports, the admin inbox, tag moderation and account notices
> work through the web board, the API and the CLI. Encrypted sessions and federation come in later phases. Not
> ready for real use.

## Workspace

| Crate | Kind | Role |
| --- | --- | --- |
| `quill-proto` | lib | Shared API types and validation. No cryptography. |
| `quill-mls-envelope` | lib | Reads MLS framing headers for routing; cannot decrypt. |
| `quill-board-ui` | lib | Dioxus components for the prompt board, rendered on the server. |
| `quill-server` | bin | axum server: web board, client API, (later) delivery service and federation. |
| `quill-core` | lib | Client library: API client, local profile, (later) all MLS state. |
| `quill` | bin | Minimal CLI client that exercises every server feature. |
| `quill-devtools` | bin | `quill-seed`, the demo-data seeder. Development only, never in the server image. |
| `quill-e2e` | tests | End-to-end tests: real server and real Postgres. |

**Core invariant:** `quill-server` never links MLS, HPKE or client crates.
[`scripts/check-server-deps.sh`](scripts/check-server-deps.sh) enforces this
in CI, so a decryption path cannot be added by accident.

## Try it with Docker

With Docker installed, this builds the server and starts it with a database:

```sh
docker compose up --build -d
# Open http://localhost:8080/ and register an account, then make it an admin:
docker compose exec server quill-server make-admin yourname
docker compose down        # stop; add -v to also delete the database
```

For a board full of test data instead, seed the empty database:

```sh
docker compose --profile demo run --rm seed
```

This creates 6 accounts (including `admin`), 1 to 3 personas each, 150
prompts and a few reports, and prints the logins. It also sets up demo
automated moderation: common data, scripts from the templates in a chain
for every hook, and a few posts the scripts refuse, flag or log (leave it
out with `--no-automod`). Every demo account's
password is `quill demo password`. Options such as `--prompts 300` or
`--seed 7` go after `seed`. The seeder only fills an empty database: to
start over, run `docker compose down -v` and start again.

The compose setup is for local testing only: it serves plain HTTP, which
browsers accept for the board's Secure session cookie only on `localhost`.

## Development

Requirements: the Rust toolchain pinned in `rust-toolchain.toml` (rustup
installs it automatically) and PostgreSQL 16+.

On Windows, `scripts/setup-windows.ps1` installs everything with Chocolatey:
the Visual Studio C++ Build Tools, NASM, Git, rustup and the pinned Rust. From
an Administrator PowerShell in the repository folder:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1 -Build
```

`-Build` also builds the CLI (`target\release\quill.exe`). The client alone
needs no database; PostgreSQL is only for running the server and its tests.

```sh
# A role that can create databases (the e2e tests create and drop their own).
createuser --createdb --pwprompt quill
createdb -O quill quill_dev

# Run the server.
QUILL_DATABASE_URL=postgres://quill:PASSWORD@localhost/quill_dev \
  cargo run -p quill-server

# Open http://localhost:8080/ for the web board, or use the CLI in another terminal.
cargo run -p quill -- server http://localhost:8080
cargo run -p quill -- register yourname
cargo run -p quill -- persona new your_handle --display-name "Your Name"
cargo run -p quill -- persona list

# Post and search prompts (Markdown bodies; tags as namespace:name).
cargo run -p quill -- prompt post --as your_handle --title "The ledger" \
  --body "A keeper finds *entries* in her own hand." \
  --maturity teen --tag genre:horror --tag "theme:Lighthouses"
cargo run -p quill -- prompt list --include genre:horror --exclude content-warning:gore
cargo run -p quill -- tags list --namespace genre

# Make an account a server administrator, then work the report inbox and tags.
# (On the board: the Reports and Server settings links.)
QUILL_DATABASE_URL=... cargo run -p quill-server -- make-admin yourname
# Reserved handles (admin, support, the server's name, your own list) only come from the server:
QUILL_DATABASE_URL=... cargo run -p quill-server -- grant-handle yourname support "Support desk"
cargo run -p quill -- admin reports
cargo run -p quill -- account notices        # what administrators did to your prompts and tags

# The CLI keeps its login and keys in an encrypted local store (keychain or passphrase).
cargo run -p quill -- store status
# Keys for a persona (needed for writing sessions), and checking anyone's.
cargo run -p quill -- keys init --as your_handle      # also publishes KeyPackages
cargo run -p quill -- keys status --as your_handle    # KeyPackages waiting; `keys publish` tops up
cargo run -p quill -- keys show some_handle
cargo run -p quill -- device list --as your_handle
# A second computer: link it from one you already use (compare the words both show).
cargo run -p quill -- device link                 # on the new computer
cargo run -p quill -- device approve 'quill-link:1:...'   # on the one in use
cargo run -p quill -- devices                     # this login's devices
cargo run -p quill -- admin namespace-set genre --required true   # every prompt needs a genre
cargo run -p quill -- admin tags --namespace theme                  # tags with usage counts
cargo run -p quill -- admin settings --new-tags approval            # writers' new tags need approval
cargo run -p quill -- admin tags --status pending                   # tag requests
cargo run -p quill -- admin namespace-set genre --color '#dbeafe' --text-color '#1e3a8a'
cargo run -p quill -- admin tag-color genre:horror --color '#7f1d1d'   # text colour automatic
```

Server configuration (environment variables):

| Variable | Default | Meaning |
| --- | --- | --- |
| `QUILL_DATABASE_URL` | required | Postgres connection URL |
| `QUILL_DOMAIN` | `localhost` | Public host name of this instance |
| `QUILL_BIND` | `127.0.0.1:8080` | Listen address |
| `QUILL_REGISTRATION_OPEN` | `true` | Allow new accounts |
| `QUILL_MAX_PERSONAS` | `20` | Personas per account |
| `QUILL_TOKEN_TTL_DAYS` | `30` | Login token lifetime |
| `QUILL_TAG_POLICY` | `open` | Starting value of the "new tags from writers" setting: `open`, `approval` or `curated`. Administrators change it in Server settings afterwards. |
| `QUILL_RESERVED_HANDLES_FILE` | none | A file of reserved persona handles, synced at every start: one per line, a word unless it starts with `=` (exact handle), `#` comments. A bad line stops the server. Admins also manage the list in Server settings → Handles. |
| `QUILL_SOURCE_URL` | this repository | Source link in the board's footer; point it at your version if you modify Quill (AGPL) |

The web board runs no JavaScript and loads nothing from other sites. Its
Content-Security-Policy allows only the board's own stylesheets (tag chip
colours come from one generated from Server settings, never inline styles), and forms are
protected by SameSite=Strict `__Host-` session cookies, per-session CSRF
tokens and a cross-site request check. Session cookies are marked Secure, so
serve the board over HTTPS; browsers make an exception for `localhost` during
development.

The CLI stores its server and login token in a profile file readable only by
you (`~/.config/quill/profile.json` on Linux; override with `QUILL_PROFILE` or
`--profile`). For scripts, `QUILL_PASSWORD` supplies the password instead of
the prompt.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
QUILL_TEST_DATABASE_URL=postgres://quill:PASSWORD@localhost/postgres cargo test --workspace
./scripts/check-server-deps.sh
cargo deny check && cargo audit && cargo vet
```

CI runs all of these on every push and pull request. Without
`QUILL_TEST_DATABASE_URL`, the end-to-end tests are skipped locally (and fail
in CI).

## Design

- [Architecture and implementation plan](docs/plan.md)
- [Protocol draft v0.1](docs/protocol-draft.md)

Security reports: see [SECURITY.md](SECURITY.md).

## Licence

Quill is licensed under the [GNU Affero General Public License v3.0](LICENSE)
(`AGPL-3.0-only`). If you run a modified Quill server for others, the AGPL
requires you to offer its users the source code of your version.
