# JSON Schemas

Schemas (draft 2020-12) for the JSON objects that travel inside encrypted
MLS messages, which the server never sees and so the
[OpenAPI file](../docs/openapi.json) doesn't cover. They are written by
hand, as the specification, and checked against the client: every object
the client produces must pass, and the examples in
[`test-vectors/json-objects.json`](../test-vectors/json-objects.json) must
give their recorded results (`cargo test -p quill-core json_object`).

| Schema | What it describes |
| --- | --- |
| [`activity.json`](activity.json) | One session message: a `Create`, `Update` or `Delete` of a note, or a `wr:Channel` named or described ([protocol draft](../docs/protocol-draft.md#5-encrypted-payload-vocabulary), section 5) |
| [`encrypted-attachment.json`](encrypted-attachment.json) | A file a note carries |
| [`device-payload.json`](device-payload.json) | One message in a login's own device group: device grants, device keys, history requests and bundles |
| [`history-rows.json`](history-rows.json) | The rows a history bundle carries |

Each describes what a sender must produce; readers ignore unknown terms,
and are lenient where a schema's description says so. Schemas for the
federated objects (`Prompt`, the session request's `Offer`, the actor
extensions) come with federation in Phase 3, with the `wr:` context
document. The `quill.example` URLs are placeholders until the project's
domain is chosen.
