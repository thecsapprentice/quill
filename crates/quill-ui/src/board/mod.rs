//! The Board section (3c): the prompt list with a reading pane, search,
//! filters, posting and editing. Its state lives for the server in view
//! (`BoardState`, provided by the frame), so filters and the open prompt
//! survive visits to other sections.

use dioxus::prelude::*;
use quill_proto::board::{
    Maturity, Prompt, PromptQuery, PromptState, Tag, TagNamespace, TagPolicy,
};
use uuid::Uuid;

use crate::{
    app::use_ui,
    backend::{Backend, Problem},
};

mod filters;
mod list;
mod post;
mod reader;

pub use list::{BoardPage, MyPromptsPage};

/// What the board asks the server for: tags to have and to hide, a lower
/// rating cap, a status and a search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub has: Vec<Tag>,
    pub hides: Vec<Tag>,
    pub maturity: Option<Maturity>,
    pub state: Option<PromptState>,
    pub search: String,
    /// `me` on My prompts: the login's own, removed ones included.
    pub author: Option<String>,
}

/// How a tag is set in the filter builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Any,
    Has,
    Hide,
}

impl Filter {
    /// The query for one page, continuing from `cursor`.
    pub fn query(&self, cursor: Option<String>) -> PromptQuery {
        let keys = |tags: &[Tag]| {
            Some(tags.iter().map(Tag::key).collect::<Vec<_>>().join(",")).filter(|s| !s.is_empty())
        };
        PromptQuery {
            include: keys(&self.has),
            exclude: keys(&self.hides),
            maturity_max: self.maturity,
            q: Some(self.search.trim().to_owned()).filter(|s| !s.is_empty()),
            state: self.state,
            author: self.author.clone(),
            cursor,
            ..PromptQuery::default()
        }
    }

    /// How many choices are set (the Filters button's count), search aside.
    pub fn count(&self) -> usize {
        self.has.len()
            + self.hides.len()
            + usize::from(self.maturity.is_some())
            + usize::from(self.state.is_some())
    }

    pub fn choice(&self, tag: &Tag) -> Choice {
        if self.has.iter().any(|t| t.key() == tag.key()) {
            Choice::Has
        } else if self.hides.iter().any(|t| t.key() == tag.key()) {
            Choice::Hide
        } else {
            Choice::Any
        }
    }

    pub fn set(&mut self, tag: &Tag, choice: Choice) {
        self.has.retain(|t| t.key() != tag.key());
        self.hides.retain(|t| t.key() != tag.key());
        match choice {
            Choice::Has => self.has.push(tag.clone()),
            Choice::Hide => self.hides.push(tag.clone()),
            Choice::Any => {}
        }
    }

    /// Clears everything but the search.
    pub fn clear(&mut self) {
        let search = std::mem::take(&mut self.search);
        let author = self.author.take();
        *self = Self {
            search,
            author,
            ..Self::default()
        };
    }

    /// The rating and status part of the summary, as "Teen or under, Open
    /// only", or `None` when neither is set.
    pub fn basics(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(m) = self.maturity {
            parts.push(format!("{} or under", quill_board_ui::maturity_label(m)));
        }
        if let Some(s) = self.state {
            parts.push(format!("{} only", state_word(s)));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

/// A status in a word, as in the filter summary.
pub fn state_word(s: PromptState) -> &'static str {
    match s {
        PromptState::Open => "Open",
        PromptState::Busy => "Busy",
        PromptState::Closed => "Closed",
    }
}

/// What the Board section shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Filters,
    /// The post form; `Some` edits that prompt.
    Post(Option<Uuid>),
}

/// The board for the server in view.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardState {
    pub view: View,
    /// The filter in use; the builder edits a copy until "Show prompts".
    pub filter: Filter,
    pub items: Vec<Prompt>,
    /// Where the next page starts; `None` once the end is reached.
    pub next: Option<String>,
    pub loading: bool,
    /// Whether the first page has arrived since the filter last changed.
    pub loaded: bool,
    pub problem: Option<Problem>,
    /// The prompt in the reading pane.
    pub open: Option<Uuid>,
    /// Bumped by each new load, so a slow, older reply is dropped.
    pub round: u32,
    pub types: Option<Vec<TagNamespace>>,
    pub policy: TagPolicy,
    /// A line to show after an action ("Prompt posted.").
    pub said: Option<String>,
    /// The board-wide change count this list last loaded at (`BoardEpoch`).
    pub seen: u32,
    /// Prompts newer than the list's, found by a check but not shown yet.
    pub fresh: usize,
    /// My prompts' persona filter. Applied here rather than by the server,
    /// whose handle filter leaves out removed prompts.
    pub persona: Option<Uuid>,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            view: View::List,
            filter: Filter::default(),
            items: Vec::new(),
            next: None,
            loading: false,
            loaded: false,
            problem: None,
            open: None,
            round: 0,
            types: None,
            policy: TagPolicy::Open,
            said: None,
            seen: 0,
            fresh: 0,
            persona: None,
        }
    }
}

/// My prompts' board, kept apart from the Board's so each keeps its own
/// list, filter and open prompt.
#[derive(Clone, Copy)]
pub struct MineState(pub Signal<BoardState>);

/// Counts changes the writer made to prompts (posting, editing, a status,
/// deleting) from either list. A list that last loaded at an older count
/// loads again on its next visit, so the Board and My prompts agree.
#[derive(Clone, Copy)]
pub struct BoardEpoch(pub Signal<u32>);

impl BoardEpoch {
    /// A change made from `board`, which already shows it.
    pub fn changed(mut self, mut board: Signal<BoardState>) {
        let next = self.0.peek().wrapping_add(1);
        self.0.set(next);
        board.write().seen = next;
    }

    pub fn now(&self) -> u32 {
        *self.0.peek()
    }
}

pub fn use_board_epoch() -> BoardEpoch {
    use_context::<BoardEpoch>()
}

impl BoardState {
    /// The board of the login's own prompts.
    pub fn mine() -> Self {
        let mut b = Self::default();
        b.filter.author = Some("me".into());
        b
    }

    /// Whether this is My prompts rather than the Board.
    pub fn is_mine(&self) -> bool {
        self.filter.author.is_some()
    }

    /// The prompts to list: all, or My prompts' chosen persona's.
    pub fn shown(&self) -> Vec<Prompt> {
        self.items
            .iter()
            .filter(|p| self.persona.is_none_or(|id| p.author.id == id))
            .cloned()
            .collect()
    }

    pub fn open_prompt(&self) -> Option<&Prompt> {
        self.items.iter().find(|p| Some(p.id) == self.open)
    }

    /// A prompt changed (edited, or its status set): updates it in place.
    pub fn replace(&mut self, prompt: Prompt) {
        if let Some(p) = self.items.iter_mut().find(|p| p.id == prompt.id) {
            *p = prompt;
        }
    }

    /// A prompt is gone (deleted here or elsewhere).
    pub fn forget(&mut self, id: Uuid) {
        self.items.retain(|p| p.id != id);
        if self.open == Some(id) {
            self.open = None;
        }
    }

    pub fn type_of(&self, namespace: &str) -> Option<&TagNamespace> {
        self.types.as_ref()?.iter().find(|t| t.name == namespace)
    }
}

/// The board's state, provided by the frame for the server in view.
pub fn use_board() -> Signal<BoardState> {
    use_context::<Signal<BoardState>>()
}

/// Loads the first page for the current filter, replacing the list.
pub fn reload(backend: std::rc::Rc<dyn Backend>, mut board: Signal<BoardState>) {
    let (query, round) = {
        let mut b = board.write();
        b.round = b.round.wrapping_add(1);
        b.loading = true;
        b.problem = None;
        (b.filter.query(None), b.round)
    };
    // The app's task, not the caller's: "Show prompts" closes the screen
    // that asked before the reply comes.
    dioxus::dioxus_core::spawn_forever(async move {
        let result = backend.prompts(query).await;
        let mut b = board.write();
        if b.round != round {
            return;
        }
        b.loading = false;
        b.loaded = true;
        match result {
            Ok(page) => {
                b.items = page.items;
                b.next = page.next_cursor;
                b.fresh = 0;
                // Keep the reading pane only if its prompt is still listed.
                if b.open_prompt().is_none() {
                    b.open = None;
                }
            }
            Err(p) => b.problem = Some(p),
        }
    });
}

/// Looks for prompts newer than the list's first page without changing
/// the list (it would move under the reader): the count is offered as
/// "N new prompts".
pub fn check_new(backend: std::rc::Rc<dyn Backend>, mut board: Signal<BoardState>) {
    let (query, round) = {
        let b = board.peek();
        if b.loading || !b.loaded {
            return;
        }
        (b.filter.query(None), b.round)
    };
    spawn(async move {
        let Ok(page) = backend.prompts(query).await else {
            // A failed check says nothing; the next one may work.
            return;
        };
        let mut b = board.write();
        if b.round != round || b.loading {
            return;
        }
        let newest = b.items.iter().map(|p| p.created_at).max();
        b.fresh = page
            .items
            .iter()
            .filter(|p| !b.items.iter().any(|q| q.id == p.id))
            .filter(|p| newest.is_none_or(|n| p.created_at >= n))
            .count();
    });
}

/// Loads the next page, appending it.
pub fn more(backend: std::rc::Rc<dyn Backend>, mut board: Signal<BoardState>) {
    let (query, round) = {
        let mut b = board.write();
        let Some(cursor) = b.next.clone() else {
            return;
        };
        if b.loading {
            return;
        }
        b.loading = true;
        b.problem = None;
        (b.filter.query(Some(cursor)), b.round)
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result = backend.prompts(query).await;
        let mut b = board.write();
        if b.round != round {
            return;
        }
        b.loading = false;
        match result {
            Ok(page) => {
                let known: Vec<Uuid> = b.items.iter().map(|p| p.id).collect();
                b.items
                    .extend(page.items.into_iter().filter(|p| !known.contains(&p.id)));
                b.next = page.next_cursor;
            }
            Err(p) => b.problem = Some(p),
        }
    });
}

/// Loads the tag types and policy once, for chips, filters and posting.
pub fn use_tag_types() {
    let (backend, _) = use_ui();
    let mut board = use_board();
    use_hook(move || {
        if board.peek().types.is_some() {
            return;
        }
        spawn(async move {
            if let Ok(types) = backend.tag_types().await {
                board.write().types = Some(types);
            }
            if let Ok(policy) = backend.tag_policy().await {
                board.write().policy = policy;
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_proto::board::{TagOrigin, TagStatus};

    fn tag(ns: &str, slug: &str) -> Tag {
        Tag {
            namespace: ns.into(),
            slug: slug.into(),
            display: slug.into(),
            origin: TagOrigin::Server,
            status: TagStatus::Visible,
            color: None,
        }
    }

    #[test]
    fn the_filter_becomes_a_query_and_counts_its_choices() {
        let mut f = Filter::default();
        assert_eq!(f.query(None), PromptQuery::default());
        f.set(&tag("genre", "horror"), Choice::Has);
        f.set(&tag("length", "long"), Choice::Has);
        f.set(&tag("content-warning", "gore"), Choice::Hide);
        f.maturity = Some(Maturity::Teen);
        f.search = "  lighthouse ".into();
        let q = f.query(Some("c".into()));
        assert_eq!(q.include.as_deref(), Some("genre:horror,length:long"));
        assert_eq!(q.exclude.as_deref(), Some("content-warning:gore"));
        assert_eq!(q.q.as_deref(), Some("lighthouse"));
        assert_eq!(q.cursor.as_deref(), Some("c"));
        assert_eq!(f.count(), 4);
        assert_eq!(f.basics().as_deref(), Some("Teen or under"));
        // One setting per tag.
        f.set(&tag("genre", "horror"), Choice::Hide);
        assert_eq!(f.choice(&tag("genre", "horror")), Choice::Hide);
        assert_eq!(f.has.len(), 1);
        f.clear();
        assert_eq!(f.count(), 0);
        assert_eq!(f.search, "  lighthouse ", "the search stays");
    }
}
