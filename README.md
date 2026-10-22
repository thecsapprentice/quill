# Quill

A federated, pseudonymous collaborative-writing server. Writers post public
**prompts** to a tag-searchable bulletin board and open end-to-end encrypted
**writing sessions** with each other. The server relays and stores session
messages but can never read them: sessions use
[Messaging Layer Security (RFC 9420)](https://www.rfc-editor.org/rfc/rfc9420.html),
and servers federate over ActivityPub.

> **Status: Phase 0 (foundations).** Accounts, personas, the client API
> skeleton and CI are in place. Prompts, encrypted sessions and federation
> come in later phases. Not ready for real use.

## Workspace

| Crate | Kind | Role |
| --- | --- | --- |
| `quill-proto` | lib | Shared API types and validation. No cryptography. |
| `quill-mls-envelope` | lib | Reads MLS framing headers for routing; cannot decrypt. |
| `quill-server` | bin | axum server: client API, (later) delivery service and federation. |
| `quill-core` | lib | Client library: API client, local profile, (later) all MLS state. |
| `quill` | bin | Minimal CLI client that exercises every server feature. |
| `quill-e2e` | tests | End-to-end tests: real server and real Postgres. |

**Core invariant:** `quill-server` never links MLS, HPKE or client crates.
[`scripts/check-server-deps.sh`](scripts/check-server-deps.sh) enforces this
in CI, so a decryption path cannot be added by accident.

## Development

Requirements: the Rust toolchain pinned in `rust-toolchain.toml` (rustup
installs it automatically) and PostgreSQL 16+.

```sh
# A role that can create databases (the e2e tests create and drop their own).
createuser --createdb --pwprompt quill
createdb -O quill quill_dev

# Run the server.
QUILL_DATABASE_URL=postgres://quill:PASSWORD@localhost/quill_dev \
  cargo run -p quill-server

# In another terminal, use the CLI.
cargo run -p quill -- server http://localhost:8080
cargo run -p quill -- register yourname
cargo run -p quill -- persona new your_handle --display-name "Your Name"
cargo run -p quill -- persona list
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
