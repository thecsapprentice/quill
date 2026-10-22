# Design documents

The design currently lives in two shared documents maintained alongside this
repository:

- **Architecture and implementation plan:** threat model, architecture, prompt
  and tag model, session cryptography (MLS), multi-device identity, federation,
  library choices, GUI framework choice, roadmap and readiness checklist.
- **Protocol draft v0.1:** the `wr:` ActivityPub vocabulary, the session
  request lifecycle, encrypted payload fields, binary formats, client API
  outline, prompt reports and conformance artifacts.

Markdown snapshots will be added to this folder so contributors can review the
design next to the code.

## Phase 0 scope

Phase 0 sets up the foundations every later phase builds on:

- Cargo workspace with the crate split from the plan, `unsafe` forbidden.
- Accounts (Argon2id password hashes, hashed bearer tokens, no email, no IP
  logging) and personas.
- `/api/v1` skeleton with RFC 9457 problem responses and body size limits.
- `quill` CLI and end-to-end tests against real Postgres.
- CI: formatting, clippy, tests, the server dependency ban, `cargo deny`,
  `cargo audit` and `cargo vet`.

Gate to leave Phase 0: CI green, protocol spec reviewed, and no MLS crypto
crate reachable from `quill-server`.
