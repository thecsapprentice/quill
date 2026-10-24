# Design documents

- [**Architecture and implementation plan**](plan.md): threat model,
  architecture, prompt and tag model, session cryptography (MLS), multi-device
  identity, federation, library choices, GUI framework choice, roadmap and
  readiness checklist.
- [**Protocol draft v0.1**](protocol-draft.md): the `wr:` ActivityPub
  vocabulary, the session request lifecycle, encrypted payload fields, binary
  formats, client API outline, prompt reports and conformance artifacts.
- [**Roadmap**](roadmap.md): every phase split into sub-phases, one pull
  request each, with status and "done when" checks.
- [**What the server keeps**](server-data.md): every column of the database,
  what it holds and why that's acceptable; the Phase 2 database audit.
- [**OpenAPI file**](openapi.json): the client API, generated from the
  server's code. With the [JSON Schemas](../schemas/) for encrypted objects
  and the [test vectors](../test-vectors/), the conformance artifacts.

Both are snapshots of a shared design document, exported on 2026-10-22.
Design changes should be made here by pull request from now on, so the design
and the code are reviewed together.
