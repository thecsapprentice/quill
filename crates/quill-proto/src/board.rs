//! Types for the prompt board: prompts, tags, account settings, reports and
//! the admin inbox.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ::serde::Serialize, ::serde::Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $text)] $variant),+
        }

        // Written out: the derive doesn't see renames made inside a macro.
        #[cfg(feature = "openapi")]
        impl ::utoipa::PartialSchema for $name {
            fn schema() -> ::utoipa::openapi::RefOr<::utoipa::openapi::schema::Schema> {
                ::utoipa::openapi::schema::ObjectBuilder::new()
                    .schema_type(::utoipa::openapi::schema::Type::String)
                    .enum_values(Some([$($text),+]))
                    .into()
            }
        }

        #[cfg(feature = "openapi")]
        impl ::utoipa::ToSchema for $name {
            fn name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(stringify!($name))
            }
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::board::UnknownValue;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s { $($text => Ok(Self::$variant),)+ _ => Err($crate::board::UnknownValue(s.to_owned())) }
            }
        }
    };
}
pub(crate) use string_enum;

/// Returned when parsing an unknown enum value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown value {0:?}")]
pub struct UnknownValue(pub String);

string_enum! {
    /// Content rating, ordered from least to most mature.
    Maturity {
        General => "general",
        Teen => "teen",
        Mature => "mature",
        Explicit => "explicit",
    }
}

impl Maturity {
    /// Cap for anonymous visitors and new accounts.
    pub const DEFAULT_MAX: Self = Self::Teen;
}

string_enum! {
    /// Whether a prompt's author is taking session requests.
    PromptState {
        Open => "open",
        Busy => "busy",
        Closed => "closed",
    }
}

string_enum! {
    /// Whether writers may create new tags by using them. A tag type marked
    /// curated-only refuses new tags whatever this says.
    TagPolicy {
        /// New tags are created as writers use them.
        Open => "open",
        /// New tags are created awaiting an administrator's approval.
        Approval => "approval",
        /// Only existing tags can be used.
        Curated => "curated",
    }
}

impl TagPolicy {
    /// Whether writers may create tags at all (perhaps pending approval).
    pub fn allows_new_tags(self) -> bool {
        !matches!(self, Self::Curated)
    }
}

string_enum! {
    /// Who can see a tag.
    TagStatus {
        /// Everyone.
        Visible => "visible",
        /// Administrators only. It stays on its prompts.
        Hidden => "hidden",
        /// A writer's new tag awaiting approval: administrators and the
        /// authors of prompts carrying it.
        Pending => "pending",
    }
}

// `string_enum!` writes the derives, so the default is implemented by hand.
#[allow(clippy::derivable_impls)]
impl Default for TagStatus {
    fn default() -> Self {
        Self::Visible
    }
}

string_enum! {
    TagOrigin {
        Server => "server",
        User => "user",
    }
}

string_enum! {
    ReportReason {
        Illegal => "illegal",
        Spam => "spam",
        Harassment => "harassment",
        WrongMaturity => "wrong-maturity",
        Other => "other",
    }
}

string_enum! {
    Resolution {
        /// Close the report; the prompt is untouched.
        Discard => "discard",
        /// Remove the prompt from the board.
        Delete => "delete",
    }
}

string_enum! {
    /// What an admin log entry records.
    AdminAction {
        ReportReceived => "report_received",
        Discard => "discard",
        DeletePrompt => "delete_prompt",
        CreateTagType => "create_tag_type",
        UpdateTagType => "update_tag_type",
        CreateTag => "create_tag",
        RenameTag => "rename_tag",
        AliasTag => "alias_tag",
        DeleteTag => "delete_tag",
        ApproveTag => "approve_tag",
        RejectTag => "reject_tag",
        HideTag => "hide_tag",
        ShowTag => "show_tag",
        ChangeSetting => "change_setting",
        RecolorTag => "recolor_tag",
        ReserveHandles => "reserve_handles",
        UnreserveHandle => "unreserve_handle",
    }
}

// ---- Tags ----------------------------------------------------------------

/// A chip's colours, as `#rrggbb`. A `None` text colour is automatic: black
/// or white, whichever contrasts more with the background.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChipColor {
    pub background: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl ChipColor {
    /// The text colour to use: the chosen one, or the automatic one.
    pub fn text_or_auto(&self) -> String {
        self.text
            .clone()
            .unwrap_or_else(|| crate::color::auto_text(&self.background).to_owned())
    }

    /// WCAG contrast ratio between the text and background (1 to 21), or
    /// `None` if either isn't a valid colour.
    pub fn contrast(&self) -> Option<f64> {
        crate::color::contrast(&self.background, &self.text_or_auto())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TagNamespace {
    pub name: String,
    pub display: String,
    /// Only server-curated tags may be used.
    pub locked: bool,
    /// Every prompt saved must carry at least one tag of this type.
    #[serde(default)]
    pub required: bool,
    /// Chip colours for this type's tags, unless a tag has its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ChipColor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub namespace: String,
    pub slug: String,
    pub display: String,
    pub origin: TagOrigin,
    /// Anything but `visible` is only ever sent to someone allowed to see it.
    #[serde(default)]
    pub status: TagStatus,
    /// The chip colours to show: the tag's own, else its type's, with the
    /// text colour resolved. `None`: no colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ChipColor>,
}

impl Tag {
    /// The `namespace:slug` form used in filters.
    pub fn key(&self) -> String {
        format!("{}:{}", self.namespace, self.slug)
    }
}

/// `GET /tags` query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct TagQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Matches the start of the slug or display name, case-insensitively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `POST /tags`: a user creates a tag in an unlocked namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateTag {
    pub namespace: String,
    pub display: String,
}

/// `POST /admin/tag-namespaces`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateNamespace {
    pub name: String,
    pub display: String,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ChipColor>,
}

/// `PATCH /admin/tag-namespaces/{name}`. Absent fields are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateNamespace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// New chip colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ChipColor>,
    /// Remove the chip colours. Ignored when `color` is given.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_color: bool,
}

/// `GET /admin/tags` query. Unlike `GET /tags`, aliases are included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct AdminTagQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Matches anywhere in the slug or display name, case-insensitively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TagStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// A tag as administrators see it: with its usage and alias target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdminTag {
    #[serde(flatten)]
    pub tag: Tag,
    /// Prompts carrying this tag, including ones removed by an administrator.
    /// Always 0 for an alias, since prompts store the tag it points to.
    pub uses: u64,
    /// `namespace:slug` of the tag this one is an alias of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<String>,
    /// The tag's own chip colours as set; `None` means it uses its type's
    /// (the result is in `tag.color`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_color: Option<ChipColor>,
}

/// Response to `DELETE /admin/tags/{namespace}/{slug}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DeletedTag {
    /// Prompts the tag was removed from.
    pub prompts_updated: u64,
}

/// `PATCH /admin/tags/{namespace}/{slug}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateTag {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// `namespace:slug` of the tag this one becomes an alias of. Prompts
    /// using it are moved to the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<String>,
    /// `visible` approves a pending tag or shows a hidden one; `hidden` hides
    /// it. `pending` can't be set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TagStatus>,
    /// The tag's own chip colours, replacing its type's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ChipColor>,
    /// Go back to the type's colours. Ignored when `color` is given.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_color: bool,
}

/// `GET /admin/settings`: settings administrators change while the server runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    /// Whether writers may create new tags.
    pub new_tags: TagPolicy,
    /// Whether sessions on this server take attachments (2i).
    #[serde(default = "yes")]
    pub attachments: bool,
}

fn yes() -> bool {
    true
}

/// `PATCH /admin/settings`. Absent fields are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateServerSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_tags: Option<TagPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<bool>,
}

// ---- Prompts -------------------------------------------------------------

/// The public identity that posted a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PromptAuthor {
    pub id: Uuid,
    pub handle: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: Uuid,
    pub author: PromptAuthor,
    pub title: String,
    /// Markdown source.
    pub body: String,
    /// Sanitized HTML rendered by the server from `body`.
    pub body_html: String,
    pub maturity: Maturity,
    pub state: PromptState,
    pub tags: Vec<Tag>,
    /// True when the author can receive session requests: they have an
    /// identity key and a full device with KeyPackages.
    pub accepting_sessions: bool,
    /// Set only in the author's own view of a prompt an administrator removed.
    #[serde(default)]
    pub removed: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// `POST /prompts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreatePrompt {
    /// Persona (owned by the caller) the prompt is posted as.
    pub persona: Uuid,
    pub title: String,
    /// Markdown.
    pub body: String,
    pub maturity: Maturity,
    /// `namespace:value`, where value is a tag's display name or slug.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PromptState>,
}

/// `PATCH /prompts/{id}`. Absent fields are unchanged; `tags` replaces the
/// whole set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePrompt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maturity: Option<Maturity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PromptState>,
}

/// `GET /prompts` query. Tag lists are comma-separated `namespace:slug`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct PromptQuery {
    /// Prompts must carry every one of these tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
    /// Prompts must carry none of these tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<String>,
    /// Lower the maturity cap for this query. It can never exceed the
    /// caller's account setting (or [`Maturity::DEFAULT_MAX`] when anonymous).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maturity_max: Option<Maturity>,
    /// Full-text search over title and body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    /// A persona handle, or `me` for all of the caller's personas (including
    /// removed prompts and ignoring the maturity cap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// `open`, `busy` or `closed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PromptState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One page of prompts, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PromptPage {
    pub items: Vec<Prompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---- Account settings ----------------------------------------------------

/// `GET /account`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AccountSettings {
    pub maturity_max: Maturity,
    pub is_admin: bool,
    /// Account notices not yet read.
    #[serde(default)]
    pub unread_notices: u64,
}

/// `PATCH /account`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccountSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maturity_max: Option<Maturity>,
}

// ---- Reports and the admin inbox -----------------------------------------

/// `POST /reports`. Anyone may report, signed in or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateReport {
    pub prompt: Uuid,
    pub reason: ReportReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// A Review Message in the administrators' inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: Uuid,
    /// `None` once the prompt itself has been deleted by its author.
    #[serde(default)]
    pub prompt: Option<Prompt>,
    pub reason: ReportReason,
    #[serde(default)]
    pub details: Option<String>,
    /// Hex SHA-256 of the prompt as it was when reported.
    pub prompt_hash: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub resolution: Option<Resolution>,
}

/// `GET /admin/reports` query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct ReportQuery {
    /// `open` (default) or `all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// `POST /admin/reports/{id}/resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResolveReport {
    pub action: Resolution,
}

/// One row of the append-only admin log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdminLogEntry {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub action: AdminAction,
    #[serde(default)]
    pub report_id: Option<Uuid>,
    #[serde(default)]
    pub prompt_id: Option<Uuid>,
    /// Hex SHA-256 of the prompt as reported (report entries only).
    #[serde(default)]
    pub prompt_hash: Option<String>,
    /// Report entries only.
    #[serde(default)]
    pub reason: Option<ReportReason>,
    /// What a tag entry changed, such as `genre:horror` or `genre`.
    #[serde(default)]
    pub subject: Option<String>,
    /// Free-form detail, such as the new name or how many prompts changed.
    #[serde(default)]
    pub detail: Option<String>,
    /// Account id of the acting administrator, if any (and not deleted).
    #[serde(default)]
    pub admin: Option<Uuid>,
    /// Who acted.
    pub actor: LogActor,
    /// The acting administrator's login name, as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_login: Option<String>,
    /// Login names of the accounts the decision was against, as they were:
    /// the first [`LOG_TARGET_PREVIEW`] alphabetically (an account being
    /// filtered for comes first). Empty for server-wide changes. The full
    /// list is at `GET /admin/log/{id}`.
    #[serde(default)]
    pub targets: Vec<String>,
    /// How many accounts the decision was against in all.
    #[serde(default)]
    pub target_count: u64,
}

/// How many affected accounts a log listing names per entry.
pub const LOG_TARGET_PREVIEW: usize = 5;

/// `GET /admin/log/{id}` query: a page of the entry's affected accounts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct AdminLogTargetQuery {
    /// Accounts after this login name, alphabetically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Default 100, at most 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One admin-log entry with a page of the accounts it was against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdminLogDetail {
    pub entry: AdminLogEntry,
    /// Alphabetical by login name.
    pub targets: Vec<AdminLogTarget>,
    /// Pass as `after` for the next page; absent on the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// An account a decision was against, and its prompts the decision touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdminLogTarget {
    /// Login name as it was.
    pub login: String,
    /// The account has since been deleted (its prompts are then unknown).
    #[serde(default)]
    pub account_deleted: bool,
    /// As they were when the decision was made. Empty for report entries,
    /// whose one prompt is the entry's own.
    #[serde(default)]
    pub prompts: Vec<AdminLogPrompt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdminLogPrompt {
    pub id: Uuid,
    pub title: String,
    /// The persona it was posted as.
    pub handle: String,
    /// Still on the server (visible or not).
    pub exists: bool,
}

string_enum! {
    /// Who made an admin-log entry.
    LogActor {
        /// An administrator, named in `adminLogin`.
        Admin => "admin",
        /// The server itself, such as syncing the reserved-handles file.
        Server => "server",
        /// A writer's report. The reporter isn't named.
        Reporter => "reporter",
    }
}

/// `GET /admin/log` query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct AdminLogQuery {
    /// Entries older than this id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Only entries by this administrator (login name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin: Option<String>,
    /// Only entries against this account (login name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maturity_orders_and_round_trips() {
        assert!(Maturity::General < Maturity::Teen);
        assert!(Maturity::Mature < Maturity::Explicit);
        for m in Maturity::ALL {
            assert_eq!(m.as_str().parse::<Maturity>().unwrap(), *m);
            assert_eq!(serde_json::to_value(m).unwrap(), m.as_str());
        }
        assert!("adult".parse::<Maturity>().is_err());
    }

    #[test]
    fn report_reason_uses_kebab_case() {
        assert_eq!(
            serde_json::to_value(ReportReason::WrongMaturity).unwrap(),
            "wrong-maturity"
        );
    }
}
