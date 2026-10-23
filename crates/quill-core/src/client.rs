//! Typed client for the `/api/v1` client API.

use quill_proto::{
    api::{
        CreatePersona, Credentials, InstanceInfo, PROBLEM_CONTENT_TYPE, Persona, Problem,
        TokenResponse, UpdatePersona,
    },
    board::{
        AccountSettings, AdminLogEntry, AdminTag, AdminTagQuery, CreateNamespace, CreatePrompt,
        CreateReport, CreateTag, DeletedTag, Prompt, PromptPage, PromptQuery, Report, ReportQuery,
        Resolution, ResolveReport, Tag, TagNamespace, TagQuery, UpdateAccountSettings,
        UpdateNamespace, UpdatePrompt, UpdateTag,
    },
};
use reqwest::{Method, RequestBuilder, Response, StatusCode, header};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Largest response body the client will read. Phase 0 responses are small;
/// this stops a hostile server from exhausting client memory.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("server URL is invalid: {0}")]
    Url(#[from] url::ParseError),
    #[error("server URL must use https (http is allowed only for localhost)")]
    InsecureUrl,
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{}", .0.detail.as_deref().unwrap_or(&.0.title))]
    Api(Problem),
    #[error("unexpected response from server ({status})")]
    Unexpected { status: StatusCode },
    #[error("response from server was too large")]
    ResponseTooLarge,
    #[error("malformed response from server: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not logged in")]
    NotLoggedIn,
    #[error("profile error: {0}")]
    Io(#[from] std::io::Error),
}

impl CoreError {
    /// The HTTP status of an API error, if this is one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api(p) => Some(p.status),
            Self::Unexpected { status } => Some(status.as_u16()),
            _ => None,
        }
    }
}

/// Checks that a server URL is safe to send credentials to.
pub fn check_server_url(raw: &str) -> Result<Url, CoreError> {
    let url = Url::parse(raw)?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    match url.scheme() {
        "https" => Ok(url),
        "http" if local => Ok(url),
        _ => Err(CoreError::InsecureUrl),
    }
}

/// An API client bound to one server and, optionally, one account token.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    token: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base.as_str())
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Client {
    /// Creates a client for `server` (e.g. `https://writers.example`).
    pub fn new(server: &str) -> Result<Self, CoreError> {
        let mut base = check_server_url(server)?;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("quill/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            base,
            token: None,
        })
    }

    /// Returns a copy of this client that sends `token` as a bearer token.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(Zeroizing::new(token.into()));
        self
    }

    pub fn server(&self) -> &Url {
        &self.base
    }

    fn url(&self, path: &str) -> Result<Url, CoreError> {
        let relative = format!(
            "{}{}",
            quill_proto::API_PREFIX.trim_start_matches('/'),
            path
        );
        Ok(self.base.join(&relative)?)
    }

    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, CoreError> {
        Ok(self.http.request(method, self.url(path)?))
    }

    fn authed(&self, method: Method, path: &str) -> Result<RequestBuilder, CoreError> {
        let token = self.token.as_ref().ok_or(CoreError::NotLoggedIn)?;
        Ok(self.request(method, path)?.bearer_auth(token.as_str()))
    }

    async fn read_body(mut response: Response) -> Result<Vec<u8>, CoreError> {
        if response
            .content_length()
            .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
        {
            return Err(CoreError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(CoreError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Turns an error response into [`CoreError::Api`] when it carries a
    /// problem document, or [`CoreError::Unexpected`] otherwise.
    async fn check(response: Response) -> Result<Response, CoreError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let is_problem = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with(PROBLEM_CONTENT_TYPE));
        let body = Self::read_body(response).await?;
        if is_problem && let Ok(problem) = serde_json::from_slice::<Problem>(&body) {
            return Err(CoreError::Api(problem));
        }
        Err(CoreError::Unexpected { status })
    }

    async fn json<T: DeserializeOwned>(builder: RequestBuilder) -> Result<T, CoreError> {
        let response = Self::check(builder.send().await?).await?;
        let body = Self::read_body(response).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    async fn empty(builder: RequestBuilder) -> Result<(), CoreError> {
        Self::check(builder.send().await?).await?;
        Ok(())
    }

    fn with_json<B: Serialize>(builder: RequestBuilder, body: &B) -> RequestBuilder {
        builder.json(body)
    }

    // ---- Instance -------------------------------------------------------

    pub async fn instance(&self) -> Result<InstanceInfo, CoreError> {
        Self::json(self.request(Method::GET, "/instance")?).await
    }

    // ---- Auth -----------------------------------------------------------

    pub async fn register(&self, creds: &Credentials) -> Result<TokenResponse, CoreError> {
        let builder = self.request(Method::POST, "/auth/register")?;
        Self::json(Self::with_json(builder, creds)).await
    }

    pub async fn login(&self, creds: &Credentials) -> Result<TokenResponse, CoreError> {
        let builder = self.request(Method::POST, "/auth/login")?;
        Self::json(Self::with_json(builder, creds)).await
    }

    pub async fn logout(&self) -> Result<(), CoreError> {
        Self::empty(self.authed(Method::POST, "/auth/logout")?).await
    }

    // ---- Personas -------------------------------------------------------

    pub async fn personas(&self) -> Result<Vec<Persona>, CoreError> {
        Self::json(self.authed(Method::GET, "/personas")?).await
    }

    pub async fn create_persona(&self, body: &CreatePersona) -> Result<Persona, CoreError> {
        let builder = self.authed(Method::POST, "/personas")?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn update_persona(
        &self,
        id: Uuid,
        body: &UpdatePersona,
    ) -> Result<Persona, CoreError> {
        let builder = self.authed(Method::PATCH, &format!("/personas/{id}"))?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn delete_persona(&self, id: Uuid) -> Result<(), CoreError> {
        Self::empty(self.authed(Method::DELETE, &format!("/personas/{id}"))?).await
    }

    /// Sends the token if this client has one; otherwise anonymous.
    fn maybe_authed(&self, method: Method, path: &str) -> Result<RequestBuilder, CoreError> {
        match &self.token {
            Some(token) => Ok(self.request(method, path)?.bearer_auth(token.as_str())),
            None => self.request(method, path),
        }
    }

    // ---- Account settings -----------------------------------------------

    pub async fn account(&self) -> Result<AccountSettings, CoreError> {
        Self::json(self.authed(Method::GET, "/account")?).await
    }

    pub async fn update_account(
        &self,
        body: &UpdateAccountSettings,
    ) -> Result<AccountSettings, CoreError> {
        let builder = self.authed(Method::PATCH, "/account")?;
        Self::json(Self::with_json(builder, body)).await
    }

    // ---- Prompts --------------------------------------------------------

    pub async fn prompts(&self, query: &PromptQuery) -> Result<PromptPage, CoreError> {
        Self::json(self.maybe_authed(Method::GET, "/prompts")?.query(query)).await
    }

    pub async fn prompt(&self, id: Uuid) -> Result<Prompt, CoreError> {
        Self::json(self.maybe_authed(Method::GET, &format!("/prompts/{id}"))?).await
    }

    pub async fn create_prompt(&self, body: &CreatePrompt) -> Result<Prompt, CoreError> {
        let builder = self.authed(Method::POST, "/prompts")?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn update_prompt(&self, id: Uuid, body: &UpdatePrompt) -> Result<Prompt, CoreError> {
        let builder = self.authed(Method::PATCH, &format!("/prompts/{id}"))?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn delete_prompt(&self, id: Uuid) -> Result<(), CoreError> {
        Self::empty(self.authed(Method::DELETE, &format!("/prompts/{id}"))?).await
    }

    // ---- Tags -----------------------------------------------------------

    pub async fn tag_namespaces(&self) -> Result<Vec<TagNamespace>, CoreError> {
        Self::json(self.request(Method::GET, "/tag-namespaces")?).await
    }

    pub async fn tags(&self, query: &TagQuery) -> Result<Vec<Tag>, CoreError> {
        Self::json(self.request(Method::GET, "/tags")?.query(query)).await
    }

    pub async fn create_tag(&self, body: &CreateTag) -> Result<Tag, CoreError> {
        let builder = self.authed(Method::POST, "/tags")?;
        Self::json(Self::with_json(builder, body)).await
    }

    // ---- Reports --------------------------------------------------------

    /// Files a report. Works signed in or anonymously.
    pub async fn report(&self, body: &CreateReport) -> Result<(), CoreError> {
        let builder = self.maybe_authed(Method::POST, "/reports")?;
        Self::empty(Self::with_json(builder, body)).await
    }

    // ---- Administration -------------------------------------------------

    pub async fn admin_reports(&self, all: bool) -> Result<Vec<Report>, CoreError> {
        let query = ReportQuery {
            state: Some(if all { "all" } else { "open" }.to_owned()),
        };
        Self::json(self.authed(Method::GET, "/admin/reports")?.query(&query)).await
    }

    pub async fn resolve_report(&self, id: Uuid, action: Resolution) -> Result<Report, CoreError> {
        let builder = self.authed(Method::POST, &format!("/admin/reports/{id}/resolve"))?;
        Self::json(Self::with_json(builder, &ResolveReport { action })).await
    }

    pub async fn admin_log(
        &self,
        before: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<AdminLogEntry>, CoreError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(b) = before {
            query.push(("before", b.to_string()));
        }
        if let Some(l) = limit {
            query.push(("limit", l.to_string()));
        }
        Self::json(self.authed(Method::GET, "/admin/log")?.query(&query)).await
    }

    pub async fn admin_create_namespace(
        &self,
        body: &CreateNamespace,
    ) -> Result<TagNamespace, CoreError> {
        let builder = self.authed(Method::POST, "/admin/tag-namespaces")?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn admin_create_tag(&self, body: &CreateTag) -> Result<Tag, CoreError> {
        let builder = self.authed(Method::POST, "/admin/tags")?;
        Self::json(Self::with_json(builder, body)).await
    }

    /// An authenticated request to `base` plus path segments, each escaped.
    fn authed_segments(
        &self,
        method: Method,
        base: &str,
        segments: &[&str],
    ) -> Result<reqwest::RequestBuilder, CoreError> {
        let mut url = self.url(base)?;
        url.path_segments_mut()
            .map_err(|()| CoreError::Url(url::ParseError::RelativeUrlWithCannotBeABaseBase))?
            .extend(segments);
        let builder = self.http.request(method, url);
        match &self.token {
            Some(token) => Ok(builder.bearer_auth(token.as_str())),
            None => Err(CoreError::NotLoggedIn),
        }
    }

    pub async fn admin_update_namespace(
        &self,
        name: &str,
        body: &UpdateNamespace,
    ) -> Result<TagNamespace, CoreError> {
        let builder = self.authed_segments(Method::PATCH, "/admin/tag-namespaces", &[name])?;
        Self::json(Self::with_json(builder, body)).await
    }

    /// Local tags with usage counts, aliases included.
    pub async fn admin_tags(&self, query: &AdminTagQuery) -> Result<Vec<AdminTag>, CoreError> {
        Self::json(self.authed(Method::GET, "/admin/tags")?.query(query)).await
    }

    pub async fn admin_update_tag(
        &self,
        namespace: &str,
        slug: &str,
        body: &UpdateTag,
    ) -> Result<Tag, CoreError> {
        let builder = self.authed_segments(Method::PATCH, "/admin/tags", &[namespace, slug])?;
        Self::json(Self::with_json(builder, body)).await
    }

    /// Deletes a tag and its aliases, removing it from every prompt.
    pub async fn admin_delete_tag(
        &self,
        namespace: &str,
        slug: &str,
    ) -> Result<DeletedTag, CoreError> {
        Self::json(self.authed_segments(Method::DELETE, "/admin/tags", &[namespace, slug])?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls() {
        assert!(check_server_url("https://writers.example").is_ok());
        assert!(check_server_url("http://localhost:8080").is_ok());
        assert!(check_server_url("http://127.0.0.1:8080").is_ok());
        assert!(matches!(
            check_server_url("http://writers.example"),
            Err(CoreError::InsecureUrl)
        ));
        assert!(check_server_url("ftp://writers.example").is_err());
        assert!(check_server_url("not a url").is_err());
    }

    #[test]
    fn api_paths() {
        let c = Client::new("https://writers.example").unwrap();
        assert_eq!(
            c.url("/personas").unwrap().as_str(),
            "https://writers.example/api/v1/personas"
        );
        let sub = Client::new("https://host.example/quill").unwrap();
        assert_eq!(
            sub.url("/instance").unwrap().as_str(),
            "https://host.example/quill/api/v1/instance"
        );
    }

    #[test]
    fn debug_hides_token() {
        let c = Client::new("https://writers.example")
            .unwrap()
            .with_token("qt_secret");
        assert!(!format!("{c:?}").contains("qt_secret"));
    }

    #[test]
    fn calls_without_token_fail_locally() {
        let c = Client::new("https://writers.example").unwrap();
        assert!(matches!(
            c.authed(Method::GET, "/personas"),
            Err(CoreError::NotLoggedIn)
        ));
    }
}
