//! The simulated server's board: tag types with limits, tags under the
//! server's tag policy, prompts, frequent tags, reports and notices. Each
//! rule follows `quill-server`'s `prompts.rs`; the messages are the
//! server's, so views show what they'd show for real.

use std::collections::HashMap;

use quill_proto::{
    board::{
        AccountSettings, ChipColor, CountedTag, CreatePrompt, CreateReport, Maturity, Prompt,
        PromptAuthor, PromptPage, PromptQuery, PromptState, Tag, TagNamespace, TagOrder, TagOrigin,
        TagPage, TagPageQuery, TagPolicy, TagQuery, TagStatus, UpdatePrompt,
    },
    limits::{DEFAULT_PAGE_SIZE, FREQUENT_TAGS, MAX_PAGE_SIZE},
    notices::{AccountNotice, NoticeFields, NoticeKind, NoticePrompt},
    validate::{self, TagSpec},
};
use uuid::Uuid;

/// Notices per page when none is asked for, as the server sends.
pub const NOTICE_PAGE: usize = 50;

use super::{Sim, SimServer};
use crate::backend::Problem;

#[derive(Debug, Clone)]
struct SimPrompt {
    prompt: Prompt,
    owner: String,
    /// `namespace:slug` of each tag, in the author's order.
    tags: Vec<String>,
    removed: bool,
    seq: u64,
}

/// One server's board.
#[derive(Debug, Clone)]
pub(super) struct Board {
    types: Vec<TagNamespace>,
    tags: Vec<Tag>,
    pub(super) policy: TagPolicy,
    prompts: Vec<SimPrompt>,
    seq: u64,
    pub(super) reports: Vec<CreateReport>,
    /// Words the server's moderation scripts refuse, with their message.
    refuse: Vec<(String, String)>,
    notices: HashMap<String, Vec<AccountNotice>>,
    notice_seq: i64,
}

fn tag(namespace: &str, display: &str, origin: TagOrigin) -> Tag {
    Tag {
        namespace: namespace.into(),
        slug: validate::tag_slug(display).unwrap_or_else(|_| display.to_lowercase()),
        display: display.into(),
        origin,
        status: TagStatus::Visible,
        color: None,
    }
}

impl Default for Board {
    /// The tag types and tags a new server has (migration 0002).
    fn default() -> Self {
        let types = [
            ("genre", "Genre", false),
            ("content-warning", "Content warning", true),
            ("format", "Format", true),
            ("length", "Length", true),
            ("theme", "Theme", false),
        ]
        .into_iter()
        .map(|(name, display, locked)| TagNamespace {
            name: name.into(),
            display: display.into(),
            locked,
            min: 0,
            max: None,
            color: None,
        })
        .collect();
        let seeded: &[(&str, &[&str])] = &[
            (
                "genre",
                &[
                    "Fantasy",
                    "Science fiction",
                    "Horror",
                    "Mystery",
                    "Romance",
                    "Thriller",
                    "Historical",
                    "Literary",
                    "Comedy",
                    "Slice of life",
                ],
            ),
            (
                "content-warning",
                &[
                    "Violence",
                    "Gore",
                    "Death",
                    "Abuse",
                    "Self-harm",
                    "Substance use",
                    "Sexual content",
                ],
            ),
            (
                "format",
                &["Prose", "Poetry", "Script", "Epistolary", "Role-play"],
            ),
            ("length", &["Flash", "Short", "Long", "Open-ended"]),
        ];
        let tags = seeded
            .iter()
            .flat_map(|(ns, names)| names.iter().map(|n| tag(ns, n, TagOrigin::Server)))
            .collect();
        Self {
            types,
            tags,
            policy: TagPolicy::Open,
            prompts: Vec::new(),
            seq: 0,
            reports: Vec::new(),
            refuse: Vec::new(),
            notices: HashMap::new(),
            notice_seq: 0,
        }
    }
}

fn bad(text: impl Into<String>) -> Problem {
    Problem::Refused(text.into())
}

fn tags_word(n: u32) -> &'static str {
    if n == 1 { "tag" } else { "tags" }
}

impl Board {
    fn tag_type(&self, name: &str) -> Option<&TagNamespace> {
        self.types.iter().find(|t| t.name == name)
    }

    /// A tag as shown: its type's colour when it has none of its own.
    fn shown(&self, key: &str) -> Option<Tag> {
        let t = self.tags.iter().find(|t| t.key() == key)?;
        let mut t = t.clone();
        if t.color.is_none() {
            t.color = self.tag_type(&t.namespace).and_then(|n| n.color.clone());
        }
        Some(t)
    }

    /// A prompt as `viewer` sees it: pending tags only for their author.
    fn hydrate(&self, p: &SimPrompt, viewer: Option<&str>) -> Prompt {
        let own = viewer == Some(p.owner.as_str());
        let mut prompt = p.prompt.clone();
        prompt.tags = p
            .tags
            .iter()
            .filter_map(|k| self.shown(k))
            .filter(|t| t.status == TagStatus::Visible || own)
            .collect();
        prompt.removed = p.removed && own;
        prompt
    }

    /// Resolves a prompt's tags, creating new ones as the policy allows
    /// (`resolve_for_write`), then checks the types' limits.
    fn resolve(&mut self, specs: &[TagSpec]) -> Result<Vec<String>, Problem> {
        let mut keys = Vec::new();
        for spec in specs {
            let key = format!("{}:{}", spec.namespace, spec.slug);
            if !self.tags.iter().any(|t| t.key() == key) {
                let missing = |why: String| {
                    bad(format!(
                        "\"{}:{}\" is not a tag on this server, and {why}",
                        spec.namespace, spec.display
                    ))
                };
                match self.tag_type(&spec.namespace).map(|n| n.locked) {
                    None => {
                        return Err(bad(format!("unknown tag namespace \"{}\"", spec.namespace)));
                    }
                    Some(true) => {
                        return Err(missing(format!(
                            "new tags can't be created in \"{}\"",
                            spec.namespace
                        )));
                    }
                    Some(false) if !self.policy.allows_new_tags() => {
                        return Err(missing("this server doesn't accept new tags".into()));
                    }
                    Some(false) => {}
                }
                let mut t = tag(&spec.namespace, &spec.display, TagOrigin::User);
                t.slug = spec.slug.clone();
                if self.policy == TagPolicy::Approval {
                    t.status = TagStatus::Pending;
                }
                self.tags.push(t);
            }
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        // The limits, as `check_tag_limits` words them.
        let mut problems = Vec::new();
        let mut types: Vec<&TagNamespace> = self
            .types
            .iter()
            .filter(|n| n.min > 0 || n.max.is_some())
            .collect();
        types.sort_by(|a, b| a.display.cmp(&b.display));
        for n in types {
            let count = keys
                .iter()
                .filter(|k| k.starts_with(&format!("{}:", n.name)))
                .count() as u32;
            if count < n.min {
                let exactly = if n.max == Some(n.min) {
                    "exactly"
                } else {
                    "at least"
                };
                problems.push(format!(
                    "Pick {exactly} {} {} {}.",
                    n.min,
                    n.display,
                    tags_word(n.min)
                ));
            } else if let Some(max) = n.max.filter(|m| count > *m) {
                problems.push(format!(
                    "Pick at most {max} {} {} (you have {count}).",
                    n.display,
                    tags_word(max)
                ));
            }
        }
        if problems.is_empty() {
            Ok(keys)
        } else {
            Err(bad(problems.join(" ")))
        }
    }

    fn moderate(&self, text: &str) -> Result<(), Problem> {
        let lower = text.to_lowercase();
        match self.refuse.iter().find(|(w, _)| lower.contains(w)) {
            Some((_, message)) => Err(bad(message.clone())),
            None => Ok(()),
        }
    }
}

fn invalid(e: validate::ValidationError) -> Problem {
    bad(e.to_string())
}

/// Who's asking: the open store's login, when signed in.
fn viewer(server: &SimServer, login: Option<&str>) -> Option<String> {
    login
        .filter(|l| server.accounts.contains_key(*l))
        .map(str::to_owned)
}

impl SimServer {
    fn cap(&self, viewer: Option<&str>) -> Maturity {
        viewer
            .and_then(|l| self.accounts.get(l))
            .map_or(Maturity::DEFAULT_MAX, |a| a.maturity)
    }

    fn persona_owner(&self, persona: Uuid) -> Option<(String, PromptAuthor, bool)> {
        self.personas
            .iter()
            .find(|(_, p, _)| p.id == persona)
            .map(|(login, p, published)| {
                (
                    login.clone(),
                    PromptAuthor {
                        id: p.id,
                        handle: p.handle.clone(),
                        display_name: p.display_name.clone(),
                    },
                    *published,
                )
            })
    }

    /// Refuses writing while `login` must accept changed rules.
    fn require_accepted(&self, login: &str) -> Result<(), Problem> {
        let behind = self.rules.as_ref().is_some_and(|(_, v)| {
            self.acceptance.on_change() && self.accounts.get(login).is_some_and(|a| a.accepted < *v)
        });
        if behind {
            return Err(Problem::RulesNotAccepted(
                "The server rules have changed. Read and accept them to continue.".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn search(
        &self,
        login: Option<&str>,
        q: &PromptQuery,
    ) -> Result<PromptPage, Problem> {
        let viewer = viewer(self, login);
        let limit = q.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE) as usize;
        let mine = q.author.as_deref() == Some("me");
        if mine && viewer.is_none() {
            return Err(Problem::SignedOut);
        }
        let cap = q.maturity_max.map_or(self.cap(viewer.as_deref()), |m| {
            m.min(self.cap(viewer.as_deref()))
        });
        let keys = |list: Option<&str>| -> Result<Vec<String>, Problem> {
            Ok(validate::tag_filter(list)
                .map_err(invalid)?
                .into_iter()
                .map(|(n, s)| format!("{n}:{s}"))
                .collect())
        };
        let include = keys(q.include.as_deref())?;
        let exclude = keys(q.exclude.as_deref())?;
        let visible = |k: &String| {
            self.board
                .tags
                .iter()
                .any(|t| t.key() == *k && t.status == TagStatus::Visible)
        };
        if !include.iter().all(visible) {
            // A required tag doesn't exist (for this viewer): nothing matches.
            return Ok(PromptPage {
                items: Vec::new(),
                next_cursor: None,
            });
        }
        let words: Vec<String> =
            q.q.as_deref()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_lowercase)
                .collect();
        let before: Option<u64> = q.cursor.as_deref().and_then(|c| c.parse().ok());
        let mut found: Vec<&SimPrompt> = self
            .board
            .prompts
            .iter()
            .filter(|p| {
                if mine {
                    Some(p.owner.as_str()) == viewer.as_deref()
                        && q.maturity_max.is_none_or(|m| p.prompt.maturity <= m)
                } else {
                    !p.removed && p.prompt.maturity <= cap
                }
            })
            .filter(|p| {
                q.author
                    .as_deref()
                    .filter(|a| *a != "me")
                    .is_none_or(|h| p.prompt.author.handle == h.to_lowercase())
            })
            .filter(|p| q.state.is_none_or(|s| p.prompt.state == s))
            .filter(|p| include.iter().all(|k| p.tags.contains(k)))
            .filter(|p| !exclude.iter().any(|k| p.tags.contains(k)))
            .filter(|p| {
                let text = format!("{} {}", p.prompt.title, p.prompt.body).to_lowercase();
                words.iter().all(|w| text.contains(w))
            })
            .filter(|p| before.is_none_or(|b| p.seq < b))
            .collect();
        found.sort_by_key(|p| std::cmp::Reverse(p.seq));
        let more = found.len() > limit;
        found.truncate(limit);
        Ok(PromptPage {
            next_cursor: more
                .then(|| found.last().map(|p| p.seq.to_string()))
                .flatten(),
            items: found
                .into_iter()
                .map(|p| self.board.hydrate(p, viewer.as_deref()))
                .collect(),
        })
    }

    pub(super) fn one(&self, login: Option<&str>, id: Uuid) -> Result<Prompt, Problem> {
        let viewer = viewer(self, login);
        self.board
            .prompts
            .iter()
            .find(|p| p.prompt.id == id && (!p.removed || Some(&p.owner) == viewer.as_ref()))
            .map(|p| self.board.hydrate(p, viewer.as_deref()))
            .ok_or(Problem::Gone)
    }

    pub(super) fn post(&mut self, login: &str, body: &CreatePrompt) -> Result<Prompt, Problem> {
        self.require_accepted(login)?;
        let title = validate::prompt_title(&body.title).map_err(invalid)?;
        let text = validate::prompt_body(&body.body).map_err(invalid)?;
        let specs = validate::tag_specs(&body.tags).map_err(invalid)?;
        let (owner, author, published) = self
            .persona_owner(body.persona)
            .filter(|(o, _, _)| o == login)
            .ok_or_else(|| bad("unknown persona"))?;
        self.board.moderate(&format!("{title}\n{text}"))?;
        let mut board = self.board.clone();
        let tags = board.resolve(&specs)?;
        self.board = board;
        self.board.seq += 1;
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(self.board.seq as i64);
        let prompt = Prompt {
            id: Uuid::new_v4(),
            author,
            title,
            body: text.clone(),
            body_html: format!("<p>{text}</p>"),
            maturity: body.maturity,
            state: body.state.unwrap_or(PromptState::Open),
            tags: Vec::new(),
            accepting_sessions: published,
            removed: false,
            created_at: now,
            updated_at: now,
        };
        let sim = SimPrompt {
            prompt,
            owner,
            tags,
            removed: false,
            seq: self.board.seq,
        };
        let shown = self.board.hydrate(&sim, Some(login));
        self.board.prompts.push(sim);
        Ok(shown)
    }

    pub(super) fn edit(
        &mut self,
        login: &str,
        id: Uuid,
        body: &UpdatePrompt,
    ) -> Result<Prompt, Problem> {
        self.require_accepted(login)?;
        let at = self
            .board
            .prompts
            .iter()
            .position(|p| p.prompt.id == id && p.owner == login)
            .ok_or(Problem::Gone)?;
        let mut p = self.board.prompts[at].clone();
        if let Some(t) = &body.title {
            p.prompt.title = validate::prompt_title(t).map_err(invalid)?;
        }
        if let Some(b) = &body.body {
            p.prompt.body = validate::prompt_body(b).map_err(invalid)?;
        }
        self.board
            .moderate(&format!("{}\n{}", p.prompt.title, p.prompt.body))?;
        if let Some(m) = body.maturity {
            p.prompt.maturity = m;
        }
        if let Some(s) = body.state {
            p.prompt.state = s;
        }
        let mut board = self.board.clone();
        if let Some(tags) = &body.tags {
            let specs = validate::tag_specs(tags).map_err(invalid)?;
            p.tags = board.resolve(&specs)?;
        }
        self.board = board;
        let shown = self.board.hydrate(&p, Some(login));
        self.board.prompts[at] = p;
        Ok(shown)
    }

    pub(super) fn delete(&mut self, login: &str, id: Uuid) -> Result<(), Problem> {
        let before = self.board.prompts.len();
        self.board
            .prompts
            .retain(|p| !(p.prompt.id == id && p.owner == login));
        if self.board.prompts.len() == before {
            return Err(Problem::Gone);
        }
        Ok(())
    }

    pub(super) fn tag_types(&self) -> Vec<TagNamespace> {
        let mut t = self.board.types.clone();
        t.sort_by(|a, b| a.name.cmp(&b.name));
        t
    }

    pub(super) fn tag_page(&self, namespace: &str, q: &TagPageQuery) -> Result<TagPage, Problem> {
        validate::namespace(namespace).map_err(invalid)?;
        let search = q.search.clone().unwrap_or_default().trim().to_lowercase();
        let letter = q
            .letter
            .filter(char::is_ascii_alphabetic)
            .map(|l| l.to_ascii_lowercase());
        let count = |t: &Tag| {
            self.board
                .prompts
                .iter()
                .filter(|p| !p.removed && p.tags.contains(&t.key()))
                .count() as u64
        };
        let mut items: Vec<CountedTag> = self
            .board
            .tags
            .iter()
            .filter(|t| t.namespace == namespace && t.status == TagStatus::Visible)
            .filter(|t| search.is_empty() || t.display.to_lowercase().contains(&search))
            .filter(|t| letter.is_none_or(|l| t.display.to_lowercase().starts_with(l)))
            .map(|t| CountedTag {
                tag: self.board.shown(&t.key()).unwrap_or_else(|| t.clone()),
                prompts: count(t),
            })
            .collect();
        match q.order.unwrap_or(TagOrder::Used) {
            TagOrder::Used => items.sort_by(|a, b| {
                b.prompts.cmp(&a.prompts).then(
                    a.tag
                        .display
                        .to_lowercase()
                        .cmp(&b.tag.display.to_lowercase()),
                )
            }),
            TagOrder::Name => {
                items.sort_by_key(|c| c.tag.display.to_lowercase());
            }
        }
        let total = items.len() as u64;
        let offset = q.offset.unwrap_or(0) as usize;
        let limit = q.limit.unwrap_or(30).clamp(1, 100) as usize;
        Ok(TagPage {
            items: items.into_iter().skip(offset).take(limit).collect(),
            total,
        })
    }

    pub(super) fn find(&self, q: &TagQuery) -> Vec<Tag> {
        let prefix = q.prefix.clone().unwrap_or_default().to_lowercase();
        let mut found: Vec<Tag> = self
            .board
            .tags
            .iter()
            .filter(|t| t.status == TagStatus::Visible)
            .filter(|t| q.namespace.as_deref().is_none_or(|n| t.namespace == n))
            .filter(|t| {
                t.slug.starts_with(&prefix) || t.display.to_lowercase().starts_with(&prefix)
            })
            .filter_map(|t| self.board.shown(&t.key()))
            .collect();
        found.sort_by_key(Tag::key);
        found.truncate(q.limit.unwrap_or(20) as usize);
        found
    }

    /// The tags `login` uses most (`frequent_tags`): over its 50 newest
    /// prompts on the board, `persona`'s only when given.
    pub(super) fn frequent(&self, login: &str, persona: Option<Uuid>) -> Vec<Tag> {
        let mut mine: Vec<&SimPrompt> = self
            .board
            .prompts
            .iter()
            .filter(|p| p.owner == login && !p.removed)
            .filter(|p| persona.is_none_or(|id| p.prompt.author.id == id))
            .collect();
        mine.sort_by_key(|p| std::cmp::Reverse(p.seq));
        mine.truncate(50);
        let mut counts: Vec<(String, usize, u64)> = Vec::new();
        for p in &mine {
            for k in &p.tags {
                match counts.iter_mut().find(|(key, _, _)| key == k) {
                    Some(c) => c.1 += 1,
                    None => counts.push((k.clone(), 1, p.seq)),
                }
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        counts
            .into_iter()
            .filter_map(|(k, _, _)| self.board.shown(&k))
            .filter(|t| t.status == TagStatus::Visible)
            .take(FREQUENT_TAGS)
            .collect()
    }

    pub(super) fn file_report(
        &mut self,
        login: Option<&str>,
        body: CreateReport,
    ) -> Result<(), Problem> {
        if let Some(l) = login {
            self.require_accepted(l)?;
        }
        if !self
            .board
            .prompts
            .iter()
            .any(|p| p.prompt.id == body.prompt)
        {
            return Err(Problem::Gone);
        }
        validate::report_details(body.details.as_deref()).map_err(invalid)?;
        self.board.reports.push(body);
        Ok(())
    }

    pub(super) fn notices_of(&self, login: &str, before: Option<i64>) -> Vec<AccountNotice> {
        let mut list: Vec<AccountNotice> = self
            .board
            .notices
            .get(login)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| before.is_none_or(|b| n.id < b))
            .collect();
        list.sort_by_key(|n| std::cmp::Reverse(n.id));
        // The server's default page.
        list.truncate(NOTICE_PAGE);
        list
    }

    pub(super) fn read_notices(&mut self, login: &str) {
        if let Some(list) = self.board.notices.get_mut(login) {
            for n in list {
                n.read = true;
            }
        }
    }

    pub(super) fn account_of(&self, login: &str) -> AccountSettings {
        AccountSettings {
            maturity_max: self.cap(Some(login)),
            is_admin: false,
            unread_notices: self
                .board
                .notices
                .get(login)
                .map_or(0, |l| l.iter().filter(|n| !n.read).count() as u64),
            staff_name: None,
            suggest_all_personas: false,
        }
    }
}

// ---- What a test sets up ---------------------------------------------------------

impl Sim {
    /// Sets a tag type's limits, as an administrator would.
    pub fn set_limits(&self, domain: &str, namespace: &str, min: u32, max: Option<u32>) {
        self.server(domain, |s| {
            let t = s
                .board
                .types
                .iter_mut()
                .find(|t| t.name == namespace)
                .unwrap_or_else(|| panic!("no tag type {namespace}"));
            t.min = min;
            t.max = max;
        });
    }

    /// Colours a tag type's chips.
    pub fn set_type_color(&self, domain: &str, namespace: &str, background: &str) {
        self.server(domain, |s| {
            if let Some(t) = s.board.types.iter_mut().find(|t| t.name == namespace) {
                t.color = Some(ChipColor {
                    background: background.into(),
                    text: None,
                });
            }
        });
    }

    pub fn set_tag_policy(&self, domain: &str, policy: TagPolicy) {
        self.server(domain, |s| s.board.policy = policy);
    }

    /// The server's moderation refuses text containing `word`.
    pub fn refuse_word(&self, domain: &str, word: &str, message: &str) {
        self.server(domain, |s| {
            s.board.refuse.push((word.to_lowercase(), message.into()));
        });
    }

    /// Another writer posts a prompt as `handle` (made if new). Returns
    /// its id.
    pub fn post_as(
        &self,
        domain: &str,
        handle: &str,
        title: &str,
        body: &str,
        tags: &[&str],
        maturity: Maturity,
    ) -> Uuid {
        let login = format!("writer_{handle}");
        self.server(domain, |s| {
            if !s.personas.iter().any(|(_, p, _)| p.handle == handle) {
                s.accounts.insert(
                    login.clone(),
                    super::Account {
                        password: "a long password".into(),
                        accepted: u32::MAX,
                        maturity: Maturity::Explicit,
                    },
                );
                s.personas
                    .push((login.clone(), super::persona(handle), true));
            }
            let persona = s
                .personas
                .iter()
                .find(|(_, p, _)| p.handle == handle)
                .map(|(o, p, _)| (o.clone(), p.id))
                .expect("the persona");
            s.post(
                &persona.0,
                &CreatePrompt {
                    persona: persona.1,
                    title: title.into(),
                    body: body.into(),
                    maturity,
                    tags: tags.iter().map(|t| (*t).to_owned()).collect(),
                    state: None,
                },
            )
            .unwrap_or_else(|p| panic!("posting {title}: {p}"))
            .id
        })
    }

    /// Several prompts by other writers, newest last: a board to browse.
    pub fn fill_board(&self, domain: &str, count: usize) {
        let genres = [
            "genre:Horror",
            "genre:Mystery",
            "genre:Fantasy",
            "genre:Romance",
        ];
        for i in 0..count {
            self.post_as(
                domain,
                &format!("writer{}", i % 4),
                &format!("Prompt number {i}"),
                &format!("The body of prompt number {i}."),
                &[genres[i % genres.len()]],
                Maturity::General,
            );
        }
    }

    /// An administrator removes a prompt (after a report, say).
    pub fn remove_prompt(&self, domain: &str, id: Uuid) {
        self.server(domain, |s| {
            for p in &mut s.board.prompts {
                if p.prompt.id == id {
                    p.removed = true;
                }
            }
        });
    }

    /// The prompt's author deletes it (from another device, say).
    pub fn delete_elsewhere(&self, domain: &str, id: Uuid) {
        self.server(domain, |s| s.board.prompts.retain(|p| p.prompt.id != id));
    }

    /// Reports filed on a server.
    pub fn reports(&self, domain: &str) -> Vec<CreateReport> {
        let mut out = Vec::new();
        self.server(domain, |s| out = s.board.reports.clone());
        out
    }

    /// The titles of a server's prompts, newest first.
    pub fn titles(&self, domain: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.server(domain, |s| {
            let mut p = s.board.prompts.clone();
            p.sort_by_key(|x| std::cmp::Reverse(x.seq));
            out = p.into_iter().map(|p| p.prompt.title).collect();
        });
        out
    }

    /// A prompt's tag keys and state, by title.
    pub fn prompt_by_title(&self, domain: &str, title: &str) -> Option<(Vec<String>, PromptState)> {
        let mut out = None;
        self.server(domain, |s| {
            out = s
                .board
                .prompts
                .iter()
                .find(|p| p.prompt.title == title)
                .map(|p| (p.tags.clone(), p.prompt.state));
        });
        out
    }

    /// An administrator's action reaches `login` as a notice.
    pub fn notify(
        &self,
        domain: &str,
        login: &str,
        kind: NoticeKind,
        fields: NoticeFields,
        prompts: &[(&str, &str)],
    ) {
        self.server(domain, |s| {
            s.board.notice_seq += 1;
            let notice = AccountNotice {
                id: s.board.notice_seq,
                at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(s.board.notice_seq),
                kind,
                fields,
                prompts: prompts
                    .iter()
                    .map(|(title, handle)| NoticePrompt {
                        id: Uuid::new_v4(),
                        title: (*title).into(),
                        handle: (*handle).into(),
                    })
                    .collect(),
                read: false,
            };
            s.board
                .notices
                .entry(login.to_owned())
                .or_default()
                .push(notice);
        });
    }

    /// Like [`Sim::notify`], about prompts already posted (by id).
    pub fn notify_about(
        &self,
        domain: &str,
        login: &str,
        kind: NoticeKind,
        fields: NoticeFields,
        prompt: Uuid,
    ) {
        let mut known = None;
        self.server(domain, |s| {
            known = s
                .board
                .prompts
                .iter()
                .find(|p| p.prompt.id == prompt)
                .map(|p| (p.prompt.title.clone(), p.prompt.author.handle.clone()));
        });
        let (title, handle) = known.expect("a posted prompt");
        self.notify(domain, login, kind, fields, &[(&title, &handle)]);
        // Point the notice at the prompt itself.
        self.server(domain, |s| {
            if let Some(n) = s.board.notices.get_mut(login).and_then(|l| l.last_mut()) {
                n.prompts[0].id = prompt;
            }
        });
    }

    /// Sets a login's maturity cap.
    pub fn set_maturity(&self, domain: &str, login: &str, cap: Maturity) {
        self.server(domain, |s| {
            if let Some(a) = s.accounts.get_mut(login) {
                a.maturity = cap;
            }
        });
    }
}
