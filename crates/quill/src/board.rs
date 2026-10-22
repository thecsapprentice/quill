//! CLI commands for the prompt board: prompts, tags, account settings,
//! reports and administration.

use std::io::Read;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use quill_core::Client;
use quill_proto::board::{
    CreateNamespace, CreatePrompt, CreateReport, CreateTag, Maturity, Prompt, PromptQuery,
    PromptState, ReportReason, Resolution, TagQuery, UpdateAccountSettings, UpdatePrompt,
    UpdateTag,
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
    fn read(&self) -> Result<Option<String>> {
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
    },
    /// Create a tag namespace.
    Namespace {
        name: String,
        display: String,
        /// Only administrators can add tags to it.
        #[arg(long)]
        locked: bool,
    },
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

async fn persona_id(client: &Client, persona: &str) -> Result<Uuid> {
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
    let settings = match cmd {
        AccountCommand::Show => client.account().await?,
        AccountCommand::Set { maturity } => {
            client
                .update_account(&UpdateAccountSettings {
                    maturity_max: maturity,
                })
                .await?
        }
    };
    ctx.print(&settings, |s| {
        println!("maturity cap: {}", s.maturity_max);
        println!("administrator: {}", s.is_admin);
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
        AdminCommand::Reports { all } => {
            let reports = client.admin_reports(all).await?;
            ctx.print(&reports, |list| {
                if list.is_empty() {
                    println!("no reports");
                }
                for r in list {
                    let title = r
                        .prompt
                        .as_ref()
                        .map_or("(prompt deleted by its author)", |p| p.title.as_str());
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
        AdminCommand::Log { before, limit } => {
            let entries = client.admin_log(before, limit).await?;
            ctx.print(&entries, |list| {
                for e in list {
                    println!(
                        "{:>6}  {}  {:<16} {:<15} prompt {}  sha256 {}",
                        e.id,
                        e.at,
                        e.action,
                        e.reason,
                        e.prompt_id.map(|p| p.to_string()).unwrap_or_default(),
                        &e.prompt_hash[..12.min(e.prompt_hash.len())]
                    );
                }
            })
        }
        AdminCommand::Namespace {
            name,
            display,
            locked,
        } => {
            let ns = client
                .admin_create_namespace(&CreateNamespace {
                    name,
                    display,
                    locked,
                })
                .await?;
            ctx.print(&ns, |n| println!("created namespace {}", n.name))
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
                        alias_of: None,
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
                        display: None,
                        alias_of: Some(target.clone()),
                    },
                )
                .await?;
            ctx.print(&t, |t| println!("{} is now an alias of {target}", t.key()))
        }
    }
}
