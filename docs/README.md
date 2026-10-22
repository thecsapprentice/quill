# Design documents

- [**Architecture and implementation plan**](plan.md): threat model,
  architecture, prompt and tag model, session cryptography (MLS), multi-device
  identity, federation, library choices, GUI framework choice, roadmap and
  readiness checklist.
- [**Protocol draft v0.1**](protocol-draft.md): the `wr:` ActivityPub
  vocabulary, the session request lifecycle, encrypted payload fields, binary
  formats, client API outline, prompt reports and conformance artifacts.

Both are snapshots of a shared design document, exported on 2026-10-22.
Design changes should be made here by pull request from now on, so the design
and the code are reviewed together.

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
