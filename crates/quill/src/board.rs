//! CLI commands for the prompt board: prompts, tags, account settings,
//! reports and administration.

use std::io::Read;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use quill_core::Client;
use quill_proto::board::{
    AdminTagQuery, CreateNamespace, CreatePrompt, CreateReport, CreateTag, Maturity, Prompt,
    PromptQuery, PromptState, ReportKind, ReportReason, Resolution, TagPolicy, TagQuery, TagStatus,
    UpdateAccountSettings, UpdateNamespace, UpdatePrompt, UpdateServerSettings, UpdateTag,
};
use uuid::Uuid;

use crate::Ctx;

#[derive(Subcommand)]
pub enum AccountCommand {
    /// Show account settings.
    Show,
    /// Change account settings.
    Set {
        /// Highest maturity rating to show on the board.
        #[arg(long)]
        maturity: Option<Maturity>,
        /// Administrators only: the public name to comment under as staff.
        /// An empty string clears it.
        #[arg(long)]
        staff_name: Option<String>,
    },
    /// Show what administrators have done to your prompts and tags, and mark
    /// it read.
    Notices {
        #[arg(long)]
        before: Option<i64>,
    },
}

#[derive(Args)]
pub struct BodyArgs {
    /// Body text (Markdown).
    #[arg(long, conflicts_with = "body_file")]
    body: Option<String>,
    /// Read the body from a file, or `-` for standard input.
    #[arg(long)]
    body_file: Option<String>,
}

impl BodyArgs {
    pub(crate) fn read(&self) -> Result<Option<String>> {
        match (&self.body, &self.body_file) {
            (Some(b), _) => Ok(Some(b.clone())),
            (None, Some(path)) if path == "-" => {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                Ok(Some(s))
            }
            (None, Some(path)) => Ok(Some(
                std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
            )),
            (None, None) => Ok(None),
        }
    }
}

#[derive(Subcommand)]
pub enum PromptCommand {
    /// Post a prompt.
    Post {
        /// Persona to post as (handle or id).
        #[arg(long = "as")]
        persona: String,
        #[arg(long)]
        title: String,
        #[command(flatten)]
        body: BodyArgs,
        #[arg(long)]
        maturity: Maturity,
        /// Tag as namespace:name; repeat for more.
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        state: Option<PromptState>,
    },
    /// Search and filter the board.
    List {
        /// Required tags, comma-separated namespace:slug.
        #[arg(long)]
        include: Option<String>,
        /// Excluded tags, comma-separated namespace:slug.
        #[arg(long)]
        exclude: Option<String>,
        /// Lower the maturity cap for this search.
        #[arg(long)]
        maturity: Option<Maturity>,
        /// Full-text search.
        #[arg(long)]
        search: Option<String>,
        /// A persona handle, or `me` for your own prompts.
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        state: Option<PromptState>,
        #[arg(long)]
        limit: Option<u32>,
        /// Continue from a previous page.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Show one prompt.
    Show { id: Uuid },
    /// Edit a prompt you posted.
    Edit {
        id: Uuid,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        body: BodyArgs,
        #[arg(long)]
        maturity: Option<Maturity>,
        /// Replace all tags; repeat for more.
        #[arg(long = "tag", conflicts_with = "clear_tags")]
        tags: Vec<String>,
        /// Remove all tags.
        #[arg(long)]
        clear_tags: bool,
        #[arg(long)]
        state: Option<PromptState>,
    },
    /// Set whether you're taking session requests: open, busy or closed.
    State { id: Uuid, state: PromptState },
    /// Delete a prompt you posted.
    Delete { id: Uuid },
}

#[derive(Subcommand)]
pub enum TagCommand {
    /// List tag namespaces.
    Namespaces,
    /// List or search tags.
    List {
        #[arg(long)]
        namespace: Option<String>,
        /// Match the start of a tag's name.
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Create a tag (if the server allows user tags).
    Create { namespace: String, name: String },
}

#[derive(Subcommand)]
pub enum AdminCommand {
    /// Storage used by each login's sessions (sizes only), or one login's
    /// sessions.
    Storage { account: Option<Uuid> },
    /// Post, edit and delete announcements.
    #[command(subcommand)]
    Announce(crate::announce_cmd::AnnounceCommand),
    /// Moderation scripts, their chains, common data, templates and dry
    /// runs.
    #[command(subcommand)]
    Automod(crate::automod_cmd::AutomodCommand),
    /// Show open reports (or all with --all).
    Reports {
        #[arg(long)]
        all: bool,
    },
    /// Resolve a report: discard it, or delete the prompt.
    Resolve { id: Uuid, action: Resolution },
    /// Read the admin log, newest first.
    Log {
        #[arg(long)]
        before: Option<i64>,
        #[arg(long)]
        limit: Option<u32>,
        /// Only this administrator's entries (login name).
        #[arg(long)]
        admin: Option<String>,
        /// Only entries against this account (login name).
        #[arg(long)]
        account: Option<String>,
    },
    /// One admin-log entry with every account it was against and the
    /// prompts it touched, a page at a time.
    LogEntry {
        id: i64,
        /// Accounts after this login name.
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Create a tag type (namespace).
    Namespace {
        name: String,
        display: String,
        /// Curated tags only: writers can't create new ones.
        #[arg(long)]
        locked: bool,
        /// Every prompt saved must have a tag of this type.
        #[arg(long)]
        required: bool,
        #[command(flatten)]
        color: ColorArgs,
    },
    /// Change a tag type's name, or whether it is curated-only or required.
    NamespaceSet {
        name: String,
        #[arg(long)]
        display: Option<String>,
        #[arg(long)]
        locked: Option<bool>,
        #[arg(long)]
        required: Option<bool>,
        #[command(flatten)]
        color: ColorArgs,
        /// Remove the chip colours.
        #[arg(long, conflicts_with = "color")]
        clear_color: bool,
    },
    /// Set a tag's own chip colours (namespace:slug), or go back to its type's.
    TagColor {
        tag: String,
        #[command(flatten)]
        color: ColorArgs,
        /// Use the tag type's colours again.
        #[arg(long, conflicts_with = "color")]
        clear: bool,
    },
    /// Show server settings, or change them.
    Settings {
        /// Whether writers may create new tags: open, approval or curated.
        #[arg(long)]
        new_tags: Option<TagPolicy>,
        /// Whether sessions take attachments: on or off.
        #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
        attachments: Option<bool>,
        /// Who can read comments on announcements: public, or signed-in
        /// (signed-in writers only).
        #[arg(long)]
        comments: Option<quill_proto::announcements::CommentVisibility>,
    },
    /// List tags with how many prompts use them, aliases included.
    Tags {
        #[arg(long)]
        namespace: Option<String>,
        /// Text the slug or display name contains.
        #[arg(long)]
        search: Option<String>,
        /// visible, hidden or pending (tag requests).
        #[arg(long)]
        status: Option<TagStatus>,
    },
    /// List reserved handles.
    Reserved {
        /// Text the entry contains.
        #[arg(long)]
        search: Option<String>,
    },
    /// Reserve handles: words (also inside handles) unless --exact.
    Reserve {
        #[arg(required = true)]
        patterns: Vec<String>,
        #[arg(long)]
        exact: bool,
    },
    /// Stop reserving a handle.
    Unreserve {
        pattern: String,
        #[arg(long)]
        exact: bool,
    },
    /// Show whether a handle is reserved, and by which entry.
    CheckHandle { handle: String },
    /// Approve a tag request (namespace:slug).
    ApproveTag { tag: String },
    /// Hide a tag: it stays on prompts, but only administrators see it.
    HideTag { tag: String },
    /// Show a hidden tag to everyone again.
    ShowTag { tag: String },
    /// Delete a tag (namespace:slug) and remove it from every prompt.
    DeleteTag { tag: String },
    /// Create a curated tag, or promote a user tag to curated.
    Tag { namespace: String, name: String },
    /// Rename a tag (namespace:slug).
    Rename { tag: String, name: String },
    /// Merge a tag into another as an alias (both namespace:slug).
    Alias { tag: String, target: String },
}

fn print_prompt(p: &Prompt, full: bool) {
    let tags: Vec<String> = p.tags.iter().map(|t| t.key()).collect();
    println!(
        "{}  {}  [{} · {}]{}",
        p.id,
        p.title,
        p.maturity,
        p.state,
        if p.removed {
            "  (removed by a server administrator)"
        } else {
            ""
        }
    );
    println!(
        "    by @{}{}",
        p.author.handle,
        if tags.is_empty() {
            String::new()
        } else {
            format!("  tags: {}", tags.join(", "))
        }
    );
    if full {
        println!();
        println!("{}", p.body);
    }
}

pub(crate) async fn persona_id(client: &Client, persona: &str) -> Result<Uuid> {
    if let Ok(id) = persona.parse() {
        return Ok(id);
    }
    let handle = persona.trim_start_matches('@').to_ascii_lowercase();
    client
        .personas()
        .await?
        .into_iter()
        .find(|p| p.handle == handle)
        .map(|p| p.id)
        .with_context(|| format!("you have no persona @{handle}"))
}

fn split_key(key: &str) -> Result<(&str, &str)> {
    key.split_once(':')
        .with_context(|| format!("{key:?} must be namespace:slug"))
}

pub async fn account(ctx: &Ctx, cmd: AccountCommand) -> Result<()> {
    let client = ctx.client()?;
    if let AccountCommand::Notices { before } = cmd {
        let notices = client.notices(before).await?;
        client.mark_notices_read().await?;
        return ctx.print(&notices, |list| {
            if list.is_empty() {
                println!("no notices");
            }
            for n in list {
                let mark = if n.read { " " } else { "*" };
                let f = &n.fields;
                let mut what = vec![n.kind.to_string()];
                what.extend(f.tag.clone());
                what.extend(f.into.as_ref().map(|i| format!("into {i}")));
                what.extend(
                    f.from
                        .as_ref()
                        .zip(f.to.as_ref())
                        .map(|(a, b)| format!("{a} -> {b}")),
                );
                what.extend(f.reason.map(|r| format!("reason: {r}")));
                println!("{mark}{:>6}  {}  {}", n.id, n.at, what.join("  "));
                for p in &n.prompts {
                    println!("          {}  {:?} as @{}", p.id, p.title, p.handle);
                }
            }
        });
    }
    let settings = match cmd {
        AccountCommand::Notices { .. } => unreachable!("handled above"),
        AccountCommand::Show => client.account().await?,
        AccountCommand::Set {
            maturity,
            staff_name,
        } => {
            client
                .update_account(&UpdateAccountSettings {
                    maturity_max: maturity,
                    staff_name,
                })
                .await?
        }
    };
    ctx.print(&settings, |s| {
        println!("maturity cap: {}", s.maturity_max);
        println!("administrator: {}", s.is_admin);
        if let Some(name) = &s.staff_name {
            println!("staff name: {name}");
        }
        println!("unread notices: {}", s.unread_notices);
    })
}

pub async fn prompt(ctx: &Ctx, cmd: PromptCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        PromptCommand::Post {
            persona,
            title,
            body,
            maturity,
            tags,
            state,
        } => {
            let Some(text) = body.read()? else {
                bail!("give the body with --body or --body-file");
            };
            let p = client
                .create_prompt(&CreatePrompt {
                    persona: persona_id(&client, &persona).await?,
                    title,
                    body: text,
                    maturity,
                    tags,
                    state,
                })
                .await?;
            ctx.print(&p, |p| print_prompt(p, false))
        }
        PromptCommand::List {
            include,
            exclude,
            maturity,
            search,
            author,
            state,
            limit,
            cursor,
        } => {
            let page = client
                .prompts(&PromptQuery {
                    include,
                    exclude,
                    maturity_max: maturity,
                    q: search,
                    author,
                    state,
                    cursor,
                    limit,
                })
                .await?;
            ctx.print(&page, |page| {
                if page.items.is_empty() {
                    println!("no prompts");
                }
                for p in &page.items {
                    print_prompt(p, false);
                }
                if let Some(c) = &page.next_cursor {
                    println!("more: --cursor {c}");
                }
            })
        }
        PromptCommand::Show { id } => {
            let p = client.prompt(id).await?;
            ctx.print(&p, |p| print_prompt(p, true))
        }
        PromptCommand::Edit {
            id,
            title,
            body,
            maturity,
            tags,
            clear_tags,
            state,
        } => {
            let tags = if clear_tags {
                Some(Vec::new())
            } else if tags.is_empty() {
                None
            } else {
                Some(tags)
            };
            let p = client
                .update_prompt(
                    id,
                    &UpdatePrompt {
                        title,
                        body: body.read()?,
                        maturity,
                        tags,
                        state,
                    },
                )
                .await?;
            ctx.print(&p, |p| print_prompt(p, false))
        }
        PromptCommand::State { id, state } => {
            let p = client
                .update_prompt(
                    id,
                    &UpdatePrompt {
                        state: Some(state),
                        ..UpdatePrompt::default()
                    },
                )
                .await?;
            ctx.print(&p, |p| println!("{} is now {}", p.id, p.state))
        }
        PromptCommand::Delete { id } => {
            client.delete_prompt(id).await?;
            ctx.done(&format!("deleted {id}"))
        }
    }
}

pub async fn tags(ctx: &Ctx, cmd: TagCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        TagCommand::Namespaces => {
            let list = client.tag_namespaces().await?;
            ctx.print(&list, |l| {
                for n in l {
                    println!(
                        "{:<20} {}{}",
                        n.name,
                        n.display,
                        if n.locked { "  (curated only)" } else { "" }
                    );
                }
            })
        }
        TagCommand::List {
            namespace,
            prefix,
            limit,
        } => {
            let list = client
                .tags(&TagQuery {
                    namespace,
                    prefix,
                    limit,
                })
                .await?;
            ctx.print(&list, |l| {
                for t in l {
                    println!("{:<36} {}  ({})", t.key(), t.display, t.origin);
                }
            })
        }
        TagCommand::Create { namespace, name } => {
            let tag = client
                .create_tag(&CreateTag {
                    namespace,
                    display: name,
                })
                .await?;
            ctx.print(&tag, |t| println!("{}  {}", t.key(), t.display))
        }
    }
}

pub async fn report(
    ctx: &Ctx,
    prompt: Uuid,
    reason: ReportReason,
    details: Option<String>,
) -> Result<()> {
    ctx.client()?
        .report(&CreateReport {
            prompt,
            reason,
            details,
        })
        .await?;
    ctx.done("report sent to the server's administrators")
}

pub async fn admin(ctx: &Ctx, cmd: AdminCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        AdminCommand::Announce(cmd) => crate::announce_cmd::announce(ctx, cmd).await,
        AdminCommand::Automod(cmd) => crate::automod_cmd::automod(ctx, cmd).await,
        AdminCommand::Reports { all } => {
            let reports = client.admin_reports(all).await?;
            ctx.print(&reports, |list| {
                if list.is_empty() {
                    println!("no reports");
                }
                for r in list {
                    let title = match (r.kind, &r.prompt, &r.comment) {
                        (ReportKind::Prompt, Some(p), _) => p.title.clone(),
                        (ReportKind::Prompt, None, _) => "(prompt deleted by its author)".into(),
                        (ReportKind::Comment, _, Some(c)) => {
                            format!("comment {} by {}", c.id, c.speaker())
                        }
                        (ReportKind::Comment, _, None) => "(comment deleted by its author)".into(),
                    };
                    let status = match (r.resolution, r.resolved_at) {
                        (Some(res), _) => format!("resolved: {res}"),
                        _ => "open".to_owned(),
                    };
                    println!("{}  {}  [{}]  {}", r.id, r.reason, status, title);
                    if let Some(d) = &r.details {
                        println!("    {d}");
                    }
                }
            })
        }
        AdminCommand::Resolve { id, action } => {
            let r = client.resolve_report(id, action).await?;
            ctx.print(&r, |r| {
                println!(
                    "report {} resolved: {}",
                    r.id,
                    r.resolution.map_or("?", Resolution::as_str)
                );
            })
        }
        AdminCommand::Log {
            before,
            limit,
            admin,
            account,
        } => {
            let entries = client
                .admin_log_filtered(&quill_proto::board::AdminLogQuery {
                    before,
                    limit,
                    admin,
                    account,
                })
                .await?;
            ctx.print(&entries, |list| {
                for e in list {
                    let what = match (&e.subject, &e.prompt_hash) {
                        (Some(subject), _) => {
                            format!("{subject}  {}", e.detail.as_deref().unwrap_or_default())
                        }
                        (None, hash) => {
                            let hash = hash.as_deref().unwrap_or_default();
                            format!(
                                "{} prompt {}  sha256 {}",
                                e.reason.map(ReportReason::as_str).unwrap_or_default(),
                                e.prompt_id.map(|p| p.to_string()).unwrap_or_default(),
                                hash.get(..12).unwrap_or(hash)
                            )
                        }
                    };
                    let who = match (e.actor, &e.admin_login) {
                        (quill_proto::board::LogActor::Admin, Some(login)) => login.clone(),
                        (actor, _) => format!("({actor})"),
                    };
                    println!("{:>6}  {}  {:<12} {:<16} {what}", e.id, e.at, who, e.action);
                    if !e.targets.is_empty() {
                        let more = usize::try_from(e.target_count)
                            .unwrap_or(usize::MAX)
                            .saturating_sub(e.targets.len());
                        let more = if more > 0 {
                            format!(" and {more} more (quill admin log-entry {})", e.id)
                        } else {
                            String::new()
                        };
                        println!("{:>8}against: {}{more}", "", e.targets.join(", "));
                    }
                }
            })
        }
        AdminCommand::LogEntry { id, after, limit } => {
            let detail = client
                .admin_log_entry(
                    id,
                    &quill_proto::board::AdminLogTargetQuery { after, limit },
                )
                .await?;
            ctx.print(&detail, |d| {
                let e = &d.entry;
                let who = match (e.actor, &e.admin_login) {
                    (quill_proto::board::LogActor::Admin, Some(login)) => login.clone(),
                    (actor, _) => format!("({actor})"),
                };
                println!("entry {}  {}  {who}  {}", e.id, e.at, e.action);
                if let Some(subject) = &e.subject {
                    println!("subject: {subject}");
                }
                if let Some(detail) = &e.detail {
                    println!("detail:  {detail}");
                }
                println!("accounts affected: {}", e.target_count);
                for t in &d.targets {
                    let gone = if t.account_deleted {
                        "  (account deleted)"
                    } else {
                        ""
                    };
                    println!("  {}{gone}", t.login);
                    for p in &t.prompts {
                        let gone = if p.exists { "" } else { "  (since removed)" };
                        println!("      {}  {} by @{}{gone}", p.id, p.title, p.handle);
                    }
                }
                if let Some(next) = &d.next {
                    println!("more: quill admin log-entry {} --after {next}", e.id);
                }
            })
        }
        AdminCommand::Namespace {
            name,
            display,
            locked,
            required,
            color,
        } => {
            let ns = client
                .admin_create_namespace(&CreateNamespace {
                    name,
                    display,
                    locked,
                    required,
                    color: color.chip(),
                })
                .await?;
            ctx.print(&ns, |n| println!("created tag type {}", n.name))
        }
        AdminCommand::TagColor { tag, color, clear } => {
            let (ns, slug) = split_key(&tag)?;
            let t = client
                .admin_update_tag(
                    ns,
                    slug,
                    &UpdateTag {
                        color: color.chip(),
                        clear_color: clear,
                        ..UpdateTag::default()
                    },
                )
                .await?;
            ctx.print(&t, |t| match &t.color {
                Some(c) => println!(
                    "{} chip: {} on {}",
                    t.key(),
                    c.text.as_deref().unwrap_or("?"),
                    c.background
                ),
                None => println!("{} has no colour", t.key()),
            })
        }
        AdminCommand::NamespaceSet {
            name,
            display,
            locked,
            required,
            color,
            clear_color,
        } => {
            let ns = client
                .admin_update_namespace(
                    &name,
                    &UpdateNamespace {
                        display,
                        locked,
                        required,
                        color: color.chip(),
                        clear_color,
                    },
                )
                .await?;
            ctx.print(&ns, |n| {
                println!(
                    "{} ({}): curated only {}, required {}",
                    n.name, n.display, n.locked, n.required
                );
            })
        }
        AdminCommand::Storage { account: None } => {
            let s = client.admin_storage().await?;
            ctx.print(&s, |s| {
                println!(
                    "{} stored: {} in messages, {} in attachments",
                    crate::size(s.total_bytes),
                    crate::size(s.message_bytes),
                    crate::size(s.file_bytes)
                );
                for a in &s.accounts {
                    println!(
                        "{}  {}  {} sessions, {} in messages, {} attachments (of {}), {} device sync",
                        a.account,
                        a.login,
                        a.sessions,
                        crate::size(a.message_bytes),
                        crate::size(a.file_bytes),
                        crate::size(a.quota_bytes),
                        crate::size(a.device_sync_bytes)
                    );
                }
            })
        }
        AdminCommand::Storage {
            account: Some(account),
        } => {
            let s = client.admin_account_storage(account).await?;
            ctx.print(&s, crate::print_storage)
        }
        AdminCommand::Settings {
            new_tags,
            attachments,
            comments,
        } => {
            let settings = if new_tags.is_some() || attachments.is_some() || comments.is_some() {
                client
                    .admin_update_settings(&UpdateServerSettings {
                        new_tags,
                        attachments,
                        comment_visibility: comments,
                        rules_acceptance: None,
                    })
                    .await?
            } else {
                client.admin_settings().await?
            };
            ctx.print(&settings, |s| {
                println!("new tags from writers: {}", s.new_tags);
                println!(
                    "attachments in sessions: {}",
                    if s.attachments {
                        "allowed"
                    } else {
                        "not allowed"
                    }
                );
                println!("who can read comments: {}", s.comment_visibility);
            })
        }
        AdminCommand::Reserved { search } => {
            let list = client
                .admin_reserved(&quill_proto::handles::ReservedQuery {
                    search,
                    ..Default::default()
                })
                .await?;
            ctx.print(&list, |l| {
                if let Some(name) = &l.server_name {
                    println!("(always reserved: the server's name, {name})");
                }
                for e in &l.entries {
                    println!("{:<32} {:<6} {}", e.pattern, e.kind, e.source);
                }
                println!("{} of {} shown", l.entries.len(), l.total);
            })
        }
        AdminCommand::Reserve { patterns, exact } => {
            let added = client
                .admin_reserve(&quill_proto::handles::AddReserved {
                    kind: reserved_kind(exact),
                    patterns,
                })
                .await?;
            ctx.print(&added, |a| println!("{} newly reserved", a.added))
        }
        AdminCommand::Unreserve { pattern, exact } => {
            client
                .admin_unreserve(reserved_kind(exact), &pattern)
                .await?;
            ctx.done(&format!("{pattern} is no longer reserved"))
        }
        AdminCommand::CheckHandle { handle } => {
            let check = client.admin_check_handle(&handle).await?;
            ctx.print(&check, |c| match &c.reserved_by {
                Some(r) => println!(
                    "@{} is reserved by {} {:?} ({})",
                    c.handle, r.kind, r.pattern, r.source
                ),
                None => println!("@{} isn't reserved", c.handle),
            })
        }
        AdminCommand::ApproveTag { tag } => {
            set_status(ctx, &tag, TagStatus::Visible, "approved").await
        }
        AdminCommand::ShowTag { tag } => set_status(ctx, &tag, TagStatus::Visible, "shown").await,
        AdminCommand::HideTag { tag } => set_status(ctx, &tag, TagStatus::Hidden, "hidden").await,
        AdminCommand::Tags {
            namespace,
            search,
            status,
        } => {
            let tags = client
                .admin_tags(&AdminTagQuery {
                    namespace,
                    search,
                    status,
                    limit: None,
                })
                .await?;
            ctx.print(&tags, |list| {
                if list.is_empty() {
                    println!("no tags");
                }
                for t in list {
                    let kind = match (&t.alias_of, t.tag.status) {
                        (Some(target), _) => format!("alias of {target}"),
                        (None, TagStatus::Visible) => t.tag.origin.to_string(),
                        (None, status) => format!("{}, {status}", t.tag.origin),
                    };
                    println!(
                        "{:<36} {:<24} {:<22} {} prompt(s)",
                        t.tag.key(),
                        t.tag.display,
                        kind,
                        t.uses
                    );
                }
            })
        }
        AdminCommand::DeleteTag { tag } => {
            let (ns, slug) = split_key(&tag)?;
            let deleted = client.admin_delete_tag(ns, slug).await?;
            ctx.print(&deleted, |d| {
                println!(
                    "deleted {tag}; removed from {} prompt(s)",
                    d.prompts_updated
                );
            })
        }
        AdminCommand::Tag { namespace, name } => {
            let tag = client
                .admin_create_tag(&CreateTag {
                    namespace,
                    display: name,
                })
                .await?;
            ctx.print(&tag, |t| println!("curated tag {}", t.key()))
        }
        AdminCommand::Rename { tag, name } => {
            let (ns, slug) = split_key(&tag)?;
            let t = client
                .admin_update_tag(
                    ns,
                    slug,
                    &UpdateTag {
                        display: Some(name),
                        ..UpdateTag::default()
                    },
                )
                .await?;
            ctx.print(&t, |t| println!("{} is now named {}", t.key(), t.display))
        }
        AdminCommand::Alias { tag, target } => {
            let (ns, slug) = split_key(&tag)?;
            let t = client
                .admin_update_tag(
                    ns,
                    slug,
                    &UpdateTag {
                        alias_of: Some(target.clone()),
                        ..UpdateTag::default()
                    },
                )
                .await?;
            ctx.print(&t, |t| println!("{} is now an alias of {target}", t.key()))
        }
    }
}

async fn set_status(ctx: &Ctx, tag: &str, status: TagStatus, done: &str) -> Result<()> {
    let (ns, slug) = split_key(tag)?;
    let t = ctx
        .client()?
        .admin_update_tag(
            ns,
            slug,
            &UpdateTag {
                status: Some(status),
                ..UpdateTag::default()
            },
        )
        .await?;
    ctx.print(&t, |t| println!("{} {done}", t.key()))
}

/// Chip colour options, as `#rrggbb`. Without `--text-color`, the text is
/// black or white, whichever reads better.
#[derive(Args, Debug)]
#[group(id = "color", multiple = true)]
pub struct ColorArgs {
    /// Chip background, like #dbeafe.
    #[arg(long = "color", value_name = "BACKGROUND")]
    background: Option<String>,
    /// Chip text colour, like #1e3a8a.
    #[arg(long = "text-color", value_name = "TEXT", requires = "background")]
    text: Option<String>,
}

impl ColorArgs {
    fn chip(&self) -> Option<quill_proto::board::ChipColor> {
        self.background
            .clone()
            .map(|background| quill_proto::board::ChipColor {
                background,
                text: self.text.clone(),
            })
    }
}

fn reserved_kind(exact: bool) -> quill_proto::handles::ReservedKind {
    if exact {
        quill_proto::handles::ReservedKind::Exact
    } else {
        quill_proto::handles::ReservedKind::Word
    }
}
