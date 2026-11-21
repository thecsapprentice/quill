[![CI](https://github.com/thecsapprentice/plait/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/thecsapprentice/plait/actions/workflows/ci.yml)

# Plait

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

## About the name

A plait is separate strands worked into one piece, and that's what writing
together is. The app keeps plain writing words on every screen; the weaving
is in its art and here:

- **Prompts** (the warp) are strung up in public, for others to weave through.
- **Requests** (shuttles) carry a writer across to someone else's prompt.
- **Sessions** (looms) are where two writers work a prompt together, in private.
- **Contributions** (strands) are what each of them adds.
- **Personas** (threads) are the names people write under, each a colour of
  its own.

Why the project was renamed from Quill, and which words appear where, is in
[the rename guide](docs/rebrand-plait.md).

## Workspace

| Crate | Kind | Role |
| --- | --- | --- |
| `plait-proto` | lib | Shared API types and validation. No cryptography. |
| `plait-mls-envelope` | lib | Reads MLS framing headers for routing; cannot decrypt. |
| `plait-board-ui` | lib | Dioxus components for the prompt board, rendered on the server. |
| `plait-server` | bin | axum server: web board, client API, (later) delivery service and federation. |
| `plait-core` | lib | Client library: API client, local profile, (later) all MLS state. |
| `plait` | bin | Minimal CLI client that exercises every server feature. |
| `plait-devtools` | bin | `plait-seed`, the demo-data seeder. Development only, never in the server image. |
| `plait-e2e` | tests | End-to-end tests: real server and real Postgres. |

**Core invariant:** `plait-server` never links MLS, HPKE or client crates.
[`scripts/check-server-deps.sh`](scripts/check-server-deps.sh) enforces this
in CI, so a decryption path cannot be added by accident.

## Try it with Docker

With Docker installed, this builds the server and starts it with a database:

```sh
docker compose up --build -d
# Open http://localhost:8080/ and register an account, then make it an admin:
docker compose exec server plait-server make-admin yourname
docker compose down        # stop; add -v to also delete the database
```

For a board full of test data instead, seed the empty database:

```sh
docker compose --profile demo run --build --rm seed
```

This creates 6 accounts (including `admin`), 1 to 3 personas each, 150
prompts and a few reports, and prints the logins. It also sets up demo
automated moderation: common data, scripts from the templates in a chain
for every hook, and a few posts the scripts refuse, flag or log (leave it
out with `--no-automod`). Every demo account's
password is `plait demo password`. Options such as `--prompts 300` or
`--seed 7` go after `seed`. The seeder only fills an empty database: to
start over, run `docker compose down -v` and start again.

The compose setup is for local testing only: it serves plain HTTP, which
browsers accept for the board's Secure session cookie only on `localhost`.

### Trying the desktop GUI

`scripts/dev-gui.py` (Python 3.8 or later, standard library only, so it runs
on Windows, macOS and Linux) does all of this in one go. It starts a fresh,
seeded server in Docker and opens the desktop GUI at its first run. When you
close the window, it stops the server and deletes its database.

```sh
python scripts/dev-gui.py                   # server on localhost:8090, then the GUI
python scripts/dev-gui.py --rules on-join   # sign-up asks to accept server rules
python scripts/dev-gui.py --guis 2          # two windows, to try two accounts at once
python scripts/dev-gui.py --no-sessions     # no demo requests or sessions; first run
python scripts/dev-gui.py --help            # the other options
```

It needs Docker and Rust. It uses its own Compose project (`plait-gui-dev`),
so the database from `docker compose up` above is left alone. In the GUI,
type `localhost:8090` as the server, then make a login or sign in as one of
the demo accounts it lists.

Each window opens signed in as a demo writer, with requests and sessions
already acted out: some waiting, accepted, declined or withdrawn, and
sessions read, unread, never opened and ended. The script prints which
writer each window is. `--no-sessions` skips this, and the GUI opens on
first run instead. Where there's no OS keychain, the windows' stores unlock
with the passphrase it prints.

With `--guis 2`, the windows are titled "Plait (1)" and "Plait (2)", each with
its own settings and stores. They also share a session and have a request
waiting for each other. The server stops once both are closed.

## Development

Requirements: the Rust toolchain pinned in `rust-toolchain.toml` (rustup
installs it automatically) and PostgreSQL 16+.

On Windows, `scripts/setup-windows.ps1` installs everything with Chocolatey:
the Visual Studio C++ Build Tools, NASM, Git, rustup and the pinned Rust. From
an Administrator PowerShell in the repository folder:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1 -Build
```

`-Build` also builds the CLI (`target\release\plait.exe`). The client alone
needs no database; PostgreSQL is only for running the server and its tests.

```sh
# A role that can create databases (the e2e tests create and drop their own).
createuser --createdb --pwprompt plait
createdb -O plait plait_dev

# Run the server.
PLAIT_DATABASE_URL=postgres://plait:PASSWORD@localhost/plait_dev \
  cargo run -p plait-server

# Open http://localhost:8080/ for the web board, or use the CLI in another terminal.
cargo run -p plait -- server http://localhost:8080
cargo run -p plait -- register yourname
cargo run -p plait -- persona new your_handle --display-name "Your Name"
cargo run -p plait -- persona list

# Post and search prompts (Markdown bodies; tags as namespace:name).
cargo run -p plait -- prompt post --as your_handle --title "The ledger" \
  --body "A keeper finds *entries* in her own hand." \
  --maturity teen --tag genre:horror --tag "theme:Lighthouses"
cargo run -p plait -- prompt list --include genre:horror --exclude content-warning:gore
cargo run -p plait -- tags list --namespace genre

# Make an account a server administrator, then work the report inbox and tags.
# (On the board: the Reports and Server settings links.)
PLAIT_DATABASE_URL=... cargo run -p plait-server -- make-admin yourname
# Reserved handles (admin, support, the server's name, your own list) only come from the server:
PLAIT_DATABASE_URL=... cargo run -p plait-server -- grant-handle yourname support "Support desk"
cargo run -p plait -- admin reports
cargo run -p plait -- account notices        # what administrators did to your prompts and tags

# The CLI keeps its login and keys in an encrypted local store (keychain or passphrase).
cargo run -p plait -- store status
# Keys for a persona (needed for writing sessions), and checking anyone's.
cargo run -p plait -- keys init --as your_handle      # also publishes KeyPackages
cargo run -p plait -- keys status --as your_handle    # KeyPackages waiting; `keys publish` tops up
cargo run -p plait -- keys show some_handle
cargo run -p plait -- device list --as your_handle
# A second computer: link it from one you already use (compare the words both show).
cargo run -p plait -- device link                 # on the new computer
cargo run -p plait -- device approve 'plait-link:1:...'   # on the one in use
cargo run -p plait -- devices                     # this login's devices
cargo run -p plait -- admin namespace-set genre --required true   # every prompt needs a genre
cargo run -p plait -- admin tags --namespace theme                  # tags with usage counts
cargo run -p plait -- admin settings --new-tags approval            # writers' new tags need approval
cargo run -p plait -- admin tags --status pending                   # tag requests
cargo run -p plait -- admin namespace-set genre --color '#dbeafe' --text-color '#1e3a8a'
cargo run -p plait -- admin tag-color genre:horror --color '#7f1d1d'   # text colour automatic
```

Server configuration (environment variables):

| Variable | Default | Meaning |
| --- | --- | --- |
| `PLAIT_DATABASE_URL` | required | Postgres connection URL |
| `PLAIT_DOMAIN` | `localhost` | Public host name of this instance |
| `PLAIT_BIND` | `127.0.0.1:8080` | Listen address |
| `PLAIT_REGISTRATION_OPEN` | `true` | Allow new accounts |
| `PLAIT_MAX_PERSONAS` | `20` | Personas per account |
| `PLAIT_TOKEN_TTL_DAYS` | `30` | Login token lifetime |
| `PLAIT_TAG_POLICY` | `open` | Starting value of the "new tags from writers" setting: `open`, `approval` or `curated`. Administrators change it in Server settings afterwards. |
| `PLAIT_RESERVED_HANDLES_FILE` | none | A file of reserved persona handles, synced at every start: one per line, a word unless it starts with `=` (exact handle), `#` comments. A bad line stops the server. Admins also manage the list in Server settings → Handles. |
| `PLAIT_SOURCE_URL` | this repository | Source link in the board's footer; point it at your version if you modify Plait (AGPL) |

The web board runs no JavaScript and loads nothing from other sites. Its
Content-Security-Policy allows only the board's own stylesheets (tag chip
colours come from one generated from Server settings, never inline styles), and forms are
protected by SameSite=Strict `__Host-` session cookies, per-session CSRF
tokens and a cross-site request check. Session cookies are marked Secure, so
serve the board over HTTPS; browsers make an exception for `localhost` during
development.

The CLI stores its server and login token in a profile file readable only by
you (`~/.config/plait/profile.json` on Linux; override with `PLAIT_PROFILE` or
`--profile`). For scripts, `PLAIT_PASSWORD` supplies the password instead of
the prompt.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
PLAIT_TEST_DATABASE_URL=postgres://plait:PASSWORD@localhost/postgres cargo test --workspace
./scripts/check-server-deps.sh
cargo deny check && cargo audit && cargo vet
```

CI runs all of these on every push and pull request. Without
`PLAIT_TEST_DATABASE_URL`, the end-to-end tests are skipped locally (and fail
in CI).

## Design

- [Architecture and implementation plan](docs/plan.md)
- [Protocol draft v0.1](docs/protocol-draft.md)

Security reports: see [SECURITY.md](SECURITY.md).

## Licence

Plait is licensed under the [GNU Affero General Public License v3.0](LICENSE)
(`AGPL-3.0-only`). If you run a modified Plait server for others, the AGPL
requires you to offer its users the source code of your version.
