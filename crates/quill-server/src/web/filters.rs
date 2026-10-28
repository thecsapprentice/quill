//! The board's filter builder at `/filters`: one GET form whose fields the
//! server merges on every request. See `quill_board_ui::filters` and
//! `docs/plan.md`, "Board layout".

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Response,
};
use quill_board_ui::{BASICS, BoardFilters, FiltersView, Nav, TagChoice, filter_keys};
use quill_proto::board::{Maturity, PromptState};

use super::{chrome, error_page, html, redirect, session::MaybeSession};
use crate::{
    AppState,
    prompts::{self, TagOrder},
};

/// Tags shown per page of a tag type.
const PAGE_SIZE: u32 = 30;

/// The last value of each field: hidden fields come first in the form, so
/// a button pressed later wins.
fn last<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.trim())
}

fn nonempty<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    last(pairs, key).filter(|v| !v.is_empty())
}

/// The filter the form describes: what the hidden fields carried, with the
/// tags that were on screen set by their radios instead.
fn merged(pairs: &[(String, String)]) -> BoardFilters {
    let shown = filter_keys(last(pairs, "shown").unwrap_or_default());
    let keep = |list: &str| -> Vec<String> {
        filter_keys(list)
            .into_iter()
            .filter(|k| !shown.contains(k))
            .collect()
    };
    let mut include = keep(last(pairs, "include").unwrap_or_default());
    let mut exclude = keep(last(pairs, "exclude").unwrap_or_default());
    for (name, value) in pairs {
        let Some(key) = name.strip_prefix("tag:") else {
            continue;
        };
        // Only tags the page showed can be set, so a crafted field can't
        // slip in something else.
        if !shown.iter().any(|k| k == key) {
            continue;
        }
        match TagChoice::parse(value) {
            Some(TagChoice::Has) => include.push(key.to_owned()),
            Some(TagChoice::Hide) => exclude.push(key.to_owned()),
            _ => {}
        }
    }
    BoardFilters {
        include: include.join(","),
        exclude: exclude.join(","),
        q: nonempty(pairs, "q").unwrap_or_default().to_owned(),
        maturity: nonempty(pairs, "maturity").and_then(|v| v.parse::<Maturity>().ok()),
        state: nonempty(pairs, "state").and_then(|v| v.parse::<PromptState>().ok()),
    }
}

pub async fn page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let filters = merged(&pairs);
    if last(&pairs, "go") == Some("apply") {
        return redirect(&filters.url(None));
    }
    let c = chrome(&state, session.as_ref(), Nav::Board, None);
    let namespaces = match prompts::namespaces(&state).await {
        Ok(n) => n,
        Err(e) => return error_page(&c, e),
    };
    // A button switching type, letter or page starts that view afresh.
    let switched = nonempty(&pairs, "to_type");
    let current = switched
        .or_else(|| nonempty(&pairs, "type"))
        .filter(|t| *t != BASICS && namespaces.iter().any(|n| n.name == *t))
        .map(str::to_owned);
    let (search, letter, page): (String, Option<char>, u32) = if switched.is_some() {
        (String::new(), None, 0)
    } else {
        let letter = match last(&pairs, "to_letter") {
            Some(l) => l.chars().next(),
            None => nonempty(&pairs, "letter").and_then(|l| l.chars().next()),
        }
        .filter(char::is_ascii_alphabetic);
        let page = if last(&pairs, "to_letter").is_some() || last(&pairs, "find").is_some() {
            0
        } else {
            nonempty(&pairs, "to_page")
                .or_else(|| nonempty(&pairs, "page"))
                .and_then(|p| p.parse().ok())
                .unwrap_or(0)
        };
        (
            last(&pairs, "tq").unwrap_or_default().to_owned(),
            letter,
            page,
        )
    };
    let by_name = last(&pairs, "order") == Some("az");
    let (tags, total) = match &current {
        Some(ns) => {
            let order = if by_name {
                TagOrder::AToZ
            } else {
                TagOrder::MostUsed
            };
            match prompts::tag_page(
                &state,
                ns,
                &search,
                letter,
                order,
                page.saturating_mul(PAGE_SIZE),
                PAGE_SIZE,
            )
            .await
            {
                Ok(found) => found,
                Err(e) => return error_page(&c, e),
            }
        }
        None => (Vec::new(), 0),
    };
    let view = FiltersView {
        filters,
        cap: session
            .as_ref()
            .map_or(Maturity::DEFAULT_MAX, |s| s.maturity_max),
        namespaces,
        current,
        search,
        letter,
        by_name,
        page,
        page_size: PAGE_SIZE,
        tags,
        total,
    };
    html(StatusCode::OK, quill_board_ui::filters_page(&c, view))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn radios_replace_what_the_hidden_fields_said_about_shown_tags() {
        let f = merged(&pairs(&[
            ("include", "genre:horror,theme:sea"),
            ("exclude", "content-warning:gore"),
            ("shown", "genre:horror,genre:fantasy"),
            ("tag:genre:horror", "any"),
            ("tag:genre:fantasy", "hide"),
            // Not on screen: ignored.
            ("tag:genre:mystery", "has"),
            ("maturity", "teen"),
        ]));
        assert_eq!(f.include, "theme:sea");
        assert_eq!(f.exclude, "content-warning:gore,genre:fantasy");
        assert_eq!(f.maturity, Some(Maturity::Teen));
        // Untouched tags of other types carry through to the board's URL.
        assert_eq!(
            f.url(None),
            "/?include=theme%3Asea&exclude=content-warning%3Agore%2Cgenre%3Afantasy&maturity=teen"
        );
    }

    #[test]
    fn the_last_value_wins() {
        let p = pairs(&[("type", "genre"), ("to_type", "theme"), ("q", "")]);
        assert_eq!(last(&p, "type"), Some("genre"));
        assert_eq!(nonempty(&p, "to_type"), Some("theme"));
        assert_eq!(nonempty(&p, "q"), None);
    }
}
