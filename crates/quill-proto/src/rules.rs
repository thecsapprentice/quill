//! The server rules (#42): one Markdown page the administrators keep.
//!
//! Each save gets the next version number. Administrators choose whether
//! accounts accept the rules on joining, after a change, both, or never;
//! a "minor edit" doesn't count as a change to accept again.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

crate::board::string_enum! {
    /// When accounts must accept the server rules: an administrators' setting.
    RulesAcceptance {
        /// Never; the rules are only there to read.
        None => "none",
        /// When creating an account.
        OnJoin => "on-join",
        /// After each change that isn't a minor edit, before writing again.
        OnChange => "on-change",
        /// Both.
        Both => "both",
    }
}

impl RulesAcceptance {
    /// Whether creating an account means accepting the rules.
    pub fn on_join(self) -> bool {
        matches!(self, Self::OnJoin | Self::Both)
    }

    /// Whether a change means accepting the rules again.
    pub fn on_change(self) -> bool {
        matches!(self, Self::OnChange | Self::Both)
    }
}

// `string_enum!` writes the derives, so the default is implemented by hand.
#[allow(clippy::derivable_impls)]
impl Default for RulesAcceptance {
    fn default() -> Self {
        Self::None
    }
}

/// `GET /rules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    /// Markdown source.
    pub body: String,
    /// Sanitized HTML rendered by the server from `body`.
    pub body_html: String,
    /// Goes up by one on every save. Accepting names the version read.
    pub version: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// For a signed-in caller: whether they must accept this version before
    /// posting, editing, commenting or reporting again. Always false otherwise.
    #[serde(default)]
    pub must_accept: bool,
}

/// `PUT /admin/rules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateRules {
    /// Markdown.
    pub body: String,
    /// A minor edit, such as a typo fix: accounts needn't accept it again.
    /// Ignored for the first version.
    #[serde(default)]
    pub minor: bool,
}

/// `POST /rules/accept`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AcceptRules {
    /// The version the caller read. Refused if the rules changed since.
    pub version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_modes() {
        assert!(!RulesAcceptance::None.on_join() && !RulesAcceptance::None.on_change());
        assert!(RulesAcceptance::OnJoin.on_join() && !RulesAcceptance::OnJoin.on_change());
        assert!(!RulesAcceptance::OnChange.on_join() && RulesAcceptance::OnChange.on_change());
        assert!(RulesAcceptance::Both.on_join() && RulesAcceptance::Both.on_change());
        assert_eq!(
            "on-change".parse::<RulesAcceptance>().unwrap(),
            RulesAcceptance::OnChange
        );
    }

    #[test]
    fn minor_defaults_to_false() {
        let u: UpdateRules = serde_json::from_str(r#"{"body":"Be kind."}"#).unwrap();
        assert!(!u.minor);
    }
}
