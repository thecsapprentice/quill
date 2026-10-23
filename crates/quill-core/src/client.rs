//! Typed client for the `/api/v1` client API.

use quill_proto::{
    api::{
        CreatePersona, Credentials, InstanceInfo, PROBLEM_CONTENT_TYPE, Persona, Problem,
        TokenResponse, UpdatePersona,
    },
    board::{
        AccountSettings, AdminLogEntry, AdminTag, AdminTagQuery, CreateNamespace, CreatePrompt,
        CreateReport, CreateTag, DeletedTag, Prompt, PromptPage, PromptQuery, Report, ReportQuery,
        Resolution, ResolveReport, ServerSettings, Tag, TagNamespace, TagQuery,
        UpdateAccountSettings, UpdateNamespace, UpdatePrompt, UpdateServerSettings, UpdateTag,
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
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    #[error("{0}")]
    Keys(String),
    #[error("certificate: {0}")]
    Cert(#[from] quill_certs::CertError),
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

    /// The account's notices: administrators' actions against its prompts
    /// and tags, newest first.
    pub async fn notices(
        &self,
        before: Option<i64>,
    ) -> Result<Vec<quill_proto::notices::AccountNotice>, CoreError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(b) = before {
            query.push(("before", b.to_string()));
        }
        Self::json(self.authed(Method::GET, "/account/notices")?.query(&query)).await
    }

    pub async fn mark_notices_read(&self) -> Result<(), CoreError> {
        Self::empty(self.authed(Method::POST, "/account/notices/read")?).await
    }

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
        self.admin_log_filtered(&quill_proto::board::AdminLogQuery {
            before,
            limit,
            ..Default::default()
        })
        .await
    }

    /// The admin log, optionally only one administrator's entries or only
    /// those against one account.
    pub async fn admin_log_filtered(
        &self,
        query: &quill_proto::board::AdminLogQuery,
    ) -> Result<Vec<AdminLogEntry>, CoreError> {
        Self::json(self.authed(Method::GET, "/admin/log")?.query(query)).await
    }

    /// One admin-log entry and a page of the accounts it was against.
    pub async fn admin_log_entry(
        &self,
        id: i64,
        query: &quill_proto::board::AdminLogTargetQuery,
    ) -> Result<quill_proto::board::AdminLogDetail, CoreError> {
        Self::json(
            self.authed(Method::GET, &format!("/admin/log/{id}"))?
                .query(query),
        )
        .await
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

    // ---- Keys and devices (2b) --------------------------------------------

    /// Sets or replaces a persona's identity key.
    pub async fn set_identity_key(
        &self,
        persona: Uuid,
        body: &quill_proto::keys::SetIdentityKey,
    ) -> Result<quill_proto::keys::PersonaKeys, CoreError> {
        let builder = self.authed(Method::PUT, &format!("/personas/{persona}/identity-key"))?;
        Self::json(Self::with_json(builder, body)).await
    }

    /// One of this account's personas' keys and devices.
    pub async fn persona_keys(
        &self,
        persona: Uuid,
    ) -> Result<quill_proto::keys::PersonaKeys, CoreError> {
        Self::json(self.authed(Method::GET, &format!("/personas/{persona}/devices"))?).await
    }

    pub async fn register_device(
        &self,
        persona: Uuid,
        body: &quill_proto::keys::RegisterDevice,
    ) -> Result<quill_proto::keys::DeviceInfo, CoreError> {
        let builder = self.authed(Method::POST, &format!("/personas/{persona}/devices"))?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn revoke_device(
        &self,
        persona: Uuid,
        device: Uuid,
        body: &quill_proto::keys::RevokeDevice,
    ) -> Result<quill_proto::keys::DeviceInfo, CoreError> {
        let builder = self.authed(
            Method::POST,
            &format!("/personas/{persona}/devices/{device}/revoke"),
        )?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn key_package_status(
        &self,
        persona: Uuid,
        device: Uuid,
    ) -> Result<quill_proto::keys::KeyPackageStatus, CoreError> {
        Self::json(self.authed(
            Method::GET,
            &format!("/personas/{persona}/devices/{device}/key-packages"),
        )?)
        .await
    }

    pub async fn upload_key_packages(
        &self,
        persona: Uuid,
        device: Uuid,
        body: &quill_proto::keys::UploadKeyPackages,
    ) -> Result<quill_proto::keys::KeyPackageStatus, CoreError> {
        let builder = self.authed(
            Method::POST,
            &format!("/personas/{persona}/devices/{device}/key-packages"),
        )?;
        Self::json(Self::with_json(builder, body)).await
    }

    /// Claims one KeyPackage per current device of a persona.
    pub async fn claim_key_packages(
        &self,
        handle: &str,
    ) -> Result<quill_proto::keys::KeyPackageClaim, CoreError> {
        self.claim_key_packages_for(handle, None).await
    }

    /// Claims for one device of the persona only, or all when `None`.
    pub async fn claim_key_packages_for(
        &self,
        handle: &str,
        device: Option<Uuid>,
    ) -> Result<quill_proto::keys::KeyPackageClaim, CoreError> {
        let builder =
            self.authed_segments(Method::POST, "/actors", &[handle, "key-packages", "claim"])?;
        Self::json(builder.query(&quill_proto::keys::ClaimQuery { device })).await
    }

    // ---- Sessions (2f) ------------------------------------------------------

    pub async fn sessions(&self) -> Result<Vec<quill_proto::sessions::Session>, CoreError> {
        Self::json(self.authed(Method::GET, "/sessions")?).await
    }

    pub async fn leave_welcome(
        &self,
        session: &str,
        body: &quill_proto::sessions::LeaveWelcome,
    ) -> Result<(), CoreError> {
        let builder = self.authed_segments(Method::POST, "/sessions", &[session, "welcomes"])?;
        Self::empty(Self::with_json(builder, body)).await
    }

    pub async fn take_welcome(&self, session: &str, device: Uuid) -> Result<(), CoreError> {
        let device = device.to_string();
        let builder =
            self.authed_segments(Method::DELETE, "/sessions", &[session, "welcomes", &device])?;
        Self::empty(builder).await
    }

    /// What the login's sessions take on the server.
    pub async fn storage(&self) -> Result<quill_proto::sessions::AccountStorage, CoreError> {
        Self::json(self.authed(Method::GET, "/account/storage")?).await
    }

    /// Administrators: every login's storage.
    pub async fn admin_storage(&self) -> Result<quill_proto::sessions::ServerStorage, CoreError> {
        Self::json(self.authed(Method::GET, "/admin/storage")?).await
    }

    /// Administrators: one login's storage, without persona handles.
    pub async fn admin_account_storage(
        &self,
        account: Uuid,
    ) -> Result<quill_proto::sessions::AccountStorage, CoreError> {
        Self::json(self.authed(Method::GET, &format!("/admin/storage/{account}"))?).await
    }

    /// Uploads a padded, encrypted blob to a session.
    pub async fn upload_blob(
        &self,
        session: &str,
        blob: Vec<u8>,
    ) -> Result<quill_proto::sessions::BlobUploaded, CoreError> {
        let builder = self
            .authed_segments(Method::POST, "/sessions", &[session, "blobs"])?
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(blob);
        Self::json(builder).await
    }

    /// A blob's URL, as attachments carry it.
    pub fn blob_url(&self, id: Uuid) -> Result<String, CoreError> {
        Ok(self.url(&format!("/blobs/{id}"))?.to_string())
    }

    /// Fetches a blob on this client's server, refusing more than `max`
    /// bytes.
    pub async fn fetch_blob(&self, url: &str, token: &str, max: u64) -> Result<Vec<u8>, CoreError> {
        let prefix = self.url("/blobs/")?.to_string();
        let id = url
            .strip_prefix(&prefix)
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| {
                CoreError::Keys(
                    "the attachment is on another server; fetching it comes with federation".into(),
                )
            })?;
        let builder = self
            .authed(Method::GET, &format!("/blobs/{id}"))?
            .header(quill_proto::sessions::BLOB_TOKEN_HEADER, token);
        let mut response = Self::check(builder.send().await?).await?;
        let max = usize::try_from(max).unwrap_or(usize::MAX);
        if response
            .content_length()
            .is_some_and(|len| len > max as u64)
        {
            return Err(CoreError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > max {
                return Err(CoreError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    pub async fn ack_session(
        &self,
        session: &str,
        body: &quill_proto::sessions::Ack,
    ) -> Result<(), CoreError> {
        let builder = self.authed_segments(Method::POST, "/sessions", &[session, "ack"])?;
        Self::empty(Self::with_json(builder, body)).await
    }

    pub async fn leave_session(&self, session: &str) -> Result<(), CoreError> {
        let builder = self.authed_segments(Method::POST, "/sessions", &[session, "leave"])?;
        Self::empty(builder).await
    }

    // ---- Groups and linking (2d) -----------------------------------------

    pub async fn create_group(
        &self,
        body: &quill_proto::groups::CreateGroup,
    ) -> Result<quill_proto::groups::GroupState, CoreError> {
        let builder = self.authed(Method::POST, "/groups")?;
        Self::json(Self::with_json(builder, body)).await
    }

    /// This login's personal device group, if it has one.
    pub async fn device_group(&self) -> Result<Option<quill_proto::groups::GroupState>, CoreError> {
        match Self::json(self.authed(Method::GET, "/account/device-group")?).await {
            Ok(g) => Ok(Some(g)),
            Err(e) if e.status() == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Appends a message to a group's log. `409` means the group has moved
    /// to a newer epoch.
    pub async fn post_group_message(
        &self,
        group: &str,
        body: &quill_proto::groups::PostMessage,
    ) -> Result<quill_proto::groups::Posted, CoreError> {
        let builder = self.authed_segments(Method::POST, "/groups", &[group, "messages"])?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn group_messages(
        &self,
        group: &str,
        query: &quill_proto::groups::MessagesQuery,
    ) -> Result<quill_proto::groups::Messages, CoreError> {
        let builder = self.authed_segments(Method::GET, "/groups", &[group, "messages"])?;
        Self::json(builder.query(query)).await
    }

    pub async fn start_link(
        &self,
        body: &quill_proto::groups::StartLink,
    ) -> Result<quill_proto::groups::LinkStarted, CoreError> {
        let builder = self.authed(Method::POST, "/link")?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn read_link(
        &self,
        link: &str,
    ) -> Result<quill_proto::groups::LinkMailbox, CoreError> {
        Self::json(self.authed_segments(Method::GET, "/link", &[link])?).await
    }

    pub async fn reply_link(
        &self,
        link: &str,
        body: &quill_proto::groups::LinkReply,
    ) -> Result<(), CoreError> {
        let builder = self.authed_segments(Method::POST, "/link", &[link, "reply"])?;
        Self::empty(Self::with_json(builder, body)).await
    }

    // ---- Requests ---------------------------------------------------------

    pub async fn send_request(
        &self,
        body: &quill_proto::requests::SendRequest,
    ) -> Result<quill_proto::requests::RequestSent, CoreError> {
        let builder = self.authed(Method::POST, "/requests")?;
        Self::json(Self::with_json(builder, body)).await
    }

    /// The login's own requests, newest first.
    pub async fn own_requests(
        &self,
    ) -> Result<Vec<quill_proto::requests::OutgoingRequest>, CoreError> {
        Self::json(self.authed(Method::GET, "/requests")?).await
    }

    pub async fn request_status(
        &self,
        id: Uuid,
    ) -> Result<quill_proto::requests::RequestState, CoreError> {
        Self::json(self.authed(Method::GET, &format!("/requests/{id}"))?).await
    }

    pub async fn withdraw_request(&self, id: Uuid) -> Result<(), CoreError> {
        Self::empty(self.authed(Method::DELETE, &format!("/requests/{id}"))?).await
    }

    /// A prompt's queue; `held` releases held requests to pending first.
    pub async fn request_queue(
        &self,
        prompt: Uuid,
        held: bool,
    ) -> Result<quill_proto::requests::Queue, CoreError> {
        let query = quill_proto::requests::QueueQuery {
            state: Some(if held { "held" } else { "pending" }.into()),
        };
        Self::json(
            self.authed(Method::GET, &format!("/prompts/{prompt}/requests"))?
                .query(&query),
        )
        .await
    }

    pub async fn accept_request(
        &self,
        id: Uuid,
    ) -> Result<quill_proto::groups::GroupState, CoreError> {
        Self::json(self.authed(Method::POST, &format!("/requests/{id}/accept"))?).await
    }

    pub async fn decline_request(&self, id: Uuid) -> Result<(), CoreError> {
        Self::empty(self.authed(Method::POST, &format!("/requests/{id}/reject"))?).await
    }

    // ---- Live delivery (2g) -------------------------------------------------

    /// Opens the login's live event stream.
    pub async fn events(&self) -> Result<EventStream, CoreError> {
        let builder = self
            .authed(Method::GET, "/events")?
            .header(header::ACCEPT, "text/event-stream");
        Ok(EventStream {
            response: Self::check(builder.send().await?).await?,
            buffer: Vec::new(),
        })
    }

    /// The server's host, as shown in link codes.
    pub fn host(&self) -> String {
        self.url("/")
            .ok()
            .and_then(|u| {
                u.host_str().map(|h| {
                    u.port()
                        .map_or_else(|| h.to_owned(), |p| format!("{h}:{p}"))
                })
            })
            .unwrap_or_default()
    }

    /// Any persona's published keys. Needs no login.
    pub async fn actor_keys(
        &self,
        handle: &str,
    ) -> Result<quill_proto::keys::PersonaKeys, CoreError> {
        let mut url = self.url("/actors")?;
        url.path_segments_mut()
            .map_err(|()| CoreError::Url(url::ParseError::RelativeUrlWithCannotBeABaseBase))?
            .extend([handle, "keys"]);
        Self::json(self.http.request(Method::GET, url)).await
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

    pub async fn admin_settings(&self) -> Result<ServerSettings, CoreError> {
        Self::json(self.authed(Method::GET, "/admin/settings")?).await
    }

    pub async fn admin_update_settings(
        &self,
        body: &UpdateServerSettings,
    ) -> Result<ServerSettings, CoreError> {
        let builder = self.authed(Method::PATCH, "/admin/settings")?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn admin_reserved(
        &self,
        query: &quill_proto::handles::ReservedQuery,
    ) -> Result<quill_proto::handles::ReservedList, CoreError> {
        Self::json(
            self.authed(Method::GET, "/admin/reserved-handles")?
                .query(query),
        )
        .await
    }

    pub async fn admin_reserve(
        &self,
        body: &quill_proto::handles::AddReserved,
    ) -> Result<quill_proto::handles::AddedReserved, CoreError> {
        let builder = self.authed(Method::POST, "/admin/reserved-handles")?;
        Self::json(Self::with_json(builder, body)).await
    }

    pub async fn admin_unreserve(
        &self,
        kind: quill_proto::handles::ReservedKind,
        pattern: &str,
    ) -> Result<(), CoreError> {
        Self::empty(self.authed_segments(
            Method::DELETE,
            "/admin/reserved-handles",
            &[kind.as_str(), pattern],
        )?)
        .await
    }

    pub async fn admin_check_handle(
        &self,
        handle: &str,
    ) -> Result<quill_proto::handles::ReservedCheck, CoreError> {
        Self::json(
            self.authed(Method::GET, "/admin/reserved-handles/check")?
                .query(&[("handle", handle)]),
        )
        .await
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

/// A live event stream (`GET /events`).
pub struct EventStream {
    response: Response,
    buffer: Vec<u8>,
}

/// The longest event the stream accepts; notices are tiny.
const MAX_EVENT_BYTES: usize = 64 * 1024;

impl EventStream {
    /// The next notice, or `None` when the server closes the stream.
    /// Notices this client doesn't know are skipped.
    pub async fn next(&mut self) -> Result<Option<quill_proto::events::Event>, CoreError> {
        loop {
            while let Some(end) = find_blank_line(&self.buffer) {
                let block: Vec<u8> = self.buffer.drain(..end.1).collect();
                if let Some(event) = parse_event(&block[..end.0]) {
                    return Ok(Some(event));
                }
            }
            match self.response.chunk().await? {
                Some(chunk) => {
                    self.buffer.extend_from_slice(&chunk);
                    if self.buffer.len() > MAX_EVENT_BYTES {
                        return Err(CoreError::ResponseTooLarge);
                    }
                }
                None => return Ok(None),
            }
        }
    }
}

/// Where the first event block ends: `(end of its text, end of the blank
/// line after it)`.
fn find_blank_line(buf: &[u8]) -> Option<(usize, usize)> {
    let text = buf;
    for (i, w) in text.windows(2).enumerate() {
        if w == b"\n\n" {
            return Some((i, i + 2));
        }
    }
    text.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, i + 4))
}

/// An event block's notice: the JSON in its `data` lines. Comments
/// (keep-alives) and unknown notices give `None`.
fn parse_event(block: &[u8]) -> Option<quill_proto::events::Event> {
    let text = std::str::from_utf8(block).ok()?;
    let data: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| d.strip_prefix(' ').unwrap_or(d))
        .collect();
    if data.is_empty() {
        return None;
    }
    serde_json::from_str(&data.join("\n")).ok()
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use quill_proto::events::Event;

    #[test]
    fn event_blocks_parse_and_keep_alives_dont() {
        assert_eq!(
            parse_event(b"event: message\ndata: {\"type\":\"message\",\"group\":\"g\",\"seq\":3}"),
            Some(Event::Message {
                group: "g".into(),
                seq: 3
            })
        );
        assert_eq!(parse_event(b":"), None);
        assert_eq!(parse_event(b"event: x\ndata: {\"type\":\"later\"}"), None);
        assert_eq!(find_blank_line(b":\n\nevent"), Some((1, 3)));
        assert_eq!(find_blank_line(b"data: x\n"), None);
    }
}
