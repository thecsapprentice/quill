//! The OpenAPI file for `/api/v1` (2m; protocol draft, section 11).
//!
//! Every API route is registered through `utoipa-axum`, which records the
//! route's documentation where it is added, so a route can't exist without
//! it. [`document`] assembles the file, and conventions every operation
//! shares (bearer auth, problem details, the common error statuses) are
//! filled in here once instead of on each handler. `docs/openapi.json` is
//! the committed copy; a test fails when it differs from what the code
//! generates (`QUILL_WRITE_OPENAPI=1` rewrites it).

use utoipa::{
    Modify, OpenApi,
    openapi::{
        Content, OpenApi as Document, RefOr, Response, ResponseBuilder,
        path::Operation,
        security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    },
};

use quill_proto::api::{PROBLEM_CONTENT_TYPE, Problem};

/// The parts of the file not tied to one route.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Quill client API",
        description = "The JSON-over-HTTPS API every Quill client uses (protocol draft, section 7). \
Federation between servers uses ActivityPub instead.\n\n\
Authenticated operations take `Authorization: Bearer <token>` from `POST /auth/register` or \
`POST /auth/login`. Errors are RFC 9457 problem details (`application/problem+json`). Every \
request body is size-checked before it is parsed: JSON bodies against the instance's \
`limits.maxEnvelopeBytes`, MLS objects inside them against `limits.maxMessageBytes`, blobs \
against `limits.maxAttachmentBytes` and backup parts against 8 MiB (`GET /instance` publishes \
the limits). Lists that page take `limit` and a cursor (`before`, `after` or `cursor`, as each \
operation says).\n\n\
Privacy: sign-in fails the same way, in the same time, whether or not the login exists; a \
requester can't tell a held, pending, declined or expired request apart; and nothing a writer \
writes in a session reaches the server except as MLS ciphertext.",
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only"),
    ),
    servers((url = "/api/v1", description = "This server")),
    components(schemas(Problem)),
    tags(
        (name = "instance", description = "What this server is and allows"),
        (name = "auth", description = "Logins and tokens"),
        (name = "personas", description = "The actors a login writes as"),
        (name = "account", description = "Settings, notices and storage of the signed-in login"),
        (name = "prompts", description = "The public board"),
        (name = "tags", description = "Tags and tag namespaces"),
        (name = "reports", description = "Reporting prompts"),
        (name = "announcements", description = "News from the server's administrators"),
        (name = "admin", description = "Administrators only"),
        (name = "keys", description = "Identity keys, devices and KeyPackages"),
        (name = "groups", description = "MLS group message logs and device linking"),
        (name = "requests", description = "Collaboration requests"),
        (name = "sessions", description = "Writing sessions"),
        (name = "blobs", description = "Encrypted attachments"),
        (name = "backup", description = "The login's encrypted backup"),
        (name = "events", description = "Live notices"),
    ),
)]
pub struct ApiDoc;

/// The scheme name operations refer to.
pub const BEARER: &str = "bearer";

/// Fills in what every operation shares.
struct Conventions;

impl Modify for Conventions {
    fn modify(&self, doc: &mut Document) {
        let components = doc.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            BEARER,
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "A token from POST /auth/register or POST /auth/login.",
                    ))
                    .build(),
            ),
        );
        for (path, item) in &mut doc.paths.paths {
            for (method, op) in [
                ("get", &mut item.get),
                ("put", &mut item.put),
                ("post", &mut item.post),
                ("patch", &mut item.patch),
                ("delete", &mut item.delete),
            ] {
                if let Some(op) = op {
                    op.operation_id = Some(operation_id(method, path));
                    conventions(op);
                }
            }
        }
    }
}

/// A unique id from the method and path: `GET /prompts/{id}/requests` is
/// `get_prompts_by_id_requests`. Handler names repeat across modules.
fn operation_id(method: &str, path: &str) -> String {
    let mut id = method.to_owned();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        id.push('_');
        match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(param) => {
                id.push_str("by_");
                id.push_str(param);
            }
            None => id.push_str(&segment.replace('-', "_")),
        }
    }
    id
}

fn problem(description: &str) -> Response {
    ResponseBuilder::new()
        .description(description)
        .content(
            PROBLEM_CONTENT_TYPE,
            Content::new(Some(RefOr::Ref(utoipa::openapi::Ref::from_schema_name(
                "Problem",
            )))),
        )
        .build()
}

fn conventions(op: &mut Operation) {
    // Public operations say `security(())`; those that take a token, or
    // optionally take one, name the bearer scheme.
    let authed = op
        .security
        .as_ref()
        .is_some_and(|s| s.iter().any(|r| r != &Default::default()));
    let takes_input =
        op.request_body.is_some() || op.parameters.as_ref().is_some_and(|p| !p.is_empty());
    let responses = &mut op.responses.responses;
    let mut add = |status: &str, description: &str| {
        responses
            .entry(status.to_owned())
            .or_insert_with(|| RefOr::T(problem(description)));
    };
    if takes_input {
        add("400", "The request is malformed or a value is invalid.");
    }
    if authed {
        add("401", "No valid bearer token.");
    }
    if op
        .tags
        .as_ref()
        .is_some_and(|t| t.iter().any(|t| t == "admin"))
    {
        add("403", "The login isn't an administrator.");
    }
    if op.request_body.is_some() {
        add("413", "The body is over the instance's size limit.");
    }
    add(
        "500",
        "Something failed on the server; the detail is only in its logs.",
    );
    // Every error is a problem document, whatever the handler listed.
    for (status, response) in responses.iter_mut() {
        if let RefOr::T(r) = response
            && status.starts_with(['4', '5'])
            && r.content.is_empty()
        {
            let description = r.description.clone();
            *r = problem(&description);
        }
        // OpenAPI requires a description on every response.
        if let RefOr::T(r) = response
            && r.description.is_empty()
        {
            r.description = match status.as_str() {
                "200" => "OK",
                "201" => "Created",
                "202" => "Accepted",
                "204" => "Done",
                _ => "See the operation's description",
            }
            .to_owned();
        }
    }
}

/// The whole file, from the API's routes.
pub fn document() -> Document {
    let (_, paths) = crate::routes::api().split_for_parts();
    let mut doc = ApiDoc::openapi();
    doc.merge(paths);
    Conventions.modify(&mut doc);
    doc
}

/// The file as committed: pretty JSON with a trailing newline.
pub fn document_json() -> String {
    let mut text = document()
        .to_pretty_json()
        .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_committed_openapi_file_is_what_the_code_generates() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/openapi.json");
        let text = document_json();
        if std::env::var_os("QUILL_WRITE_OPENAPI").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{} missing; set QUILL_WRITE_OPENAPI=1", path.display()));
        assert!(
            committed == text,
            "docs/openapi.json differs from what the code generates: an API change needs the \
             file rewritten (QUILL_WRITE_OPENAPI=1 cargo test -p quill-server openapi) and \
             reviewed"
        );
    }

    #[test]
    fn every_operation_is_documented() {
        let doc = document();
        let mut count = 0;
        for (path, item) in &doc.paths.paths {
            for (method, op) in [
                ("get", &item.get),
                ("put", &item.put),
                ("post", &item.post),
                ("patch", &item.patch),
                ("delete", &item.delete),
            ] {
                let Some(op) = op else { continue };
                count += 1;
                let name = format!("{method} {path}");
                assert!(op.summary.is_some(), "{name}: no summary");
                assert!(
                    op.tags.as_ref().is_some_and(|t| !t.is_empty()),
                    "{name}: no tag"
                );
                assert!(
                    op.responses
                        .responses
                        .keys()
                        .any(|s| s.starts_with('2') || s.starts_with('3')),
                    "{name}: no success response"
                );
                assert!(
                    op.security.is_some(),
                    "{name}: say whether it needs a token"
                );
            }
        }
        assert!(count > 80, "only {count} operations");
    }
}
