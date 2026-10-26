//! CLI commands for announcements: reading and commenting on them (anyone),
//! and posting, editing, deleting and moderating them (administrators,
//! under `quill admin announce`).

use anyhow::{Result, bail};
use clap::Subcommand;
use quill_proto::{
    announcements::{
        Announcement, AnnouncementQuery, Comment, CommentQuery, CreateAnnouncement, CreateComment,
        RemoveComment, ReportComment, UpdateAnnouncement,
    },
    board::ReportReason,
};
use uuid::Uuid;

use crate::{
    Ctx,
    board::{BodyArgs, persona_id},
};

#[derive(Subcommand)]
pub enum AnnouncementsCommand {
    /// List announcements, newest first.
    List {
        /// Continue from a previous page.
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Show one announcement in full.
    Show { id: Uuid },
    /// List the comments on an announcement, oldest first.
    Comments {
        id: Uuid,
        /// Continue from a previous page.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Comment on an announcement. Comments can't be edited afterwards.
    Comment {
        id: Uuid,
        /// Persona to comment as (handle or id).
        #[arg(long = "as")]
        persona: String,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// Delete one of your comments.
    DeleteComment { comment: Uuid },
    /// Report a comment to the server's administrators.
    ReportComment {
        comment: Uuid,
        /// illegal, spam, harassment, wrong-maturity or other
        #[arg(long)]
        reason: ReportReason,
        #[arg(long)]
        details: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AnnounceCommand {
    /// Post an announcement as the server.
    Post {
        #[arg(long)]
        title: String,
        #[command(flatten)]
        body: BodyArgs,
        /// Don't allow comments on it.
        #[arg(long)]
        no_comments: bool,
        /// Keep its banner at the top of the board, not dismissible.
        #[arg(long)]
        sticky: bool,
    },
    /// Make an announcement sticky or not.
    Sticky {
        id: Uuid,
        /// on or off.
        #[arg(value_parser = clap::builder::BoolishValueParser::new())]
        sticky: bool,
    },
    /// Change an announcement's title or text.
    Edit {
        id: Uuid,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// Delete an announcement: it shows nowhere from then on.
    Delete { id: Uuid },
    /// Remove a comment without a report. Its author is told the reason.
    RemoveComment {
        comment: Uuid,
        /// illegal, spam, harassment, wrong-maturity or other
        #[arg(long)]
        reason: ReportReason,
    },
    /// Open or close an announcement's comments.
    Comments {
        id: Uuid,
        /// on or off.
        #[arg(value_parser = clap::builder::BoolishValueParser::new())]
        open: bool,
    },
}

fn print_announcement(a: &Announcement, full: bool) {
    println!(
        "{}  {}  {}{}{}",
        a.id,
        a.created_at.date(),
        a.title,
        if a.sticky { "  (sticky)" } else { "" },
        if a.comments_open {
            ""
        } else {
            "  (comments closed)"
        }
    );
    if full {
        if a.edited() {
            println!("    edited {}", a.updated_at.date());
        }
        println!();
        println!("{}", a.body);
    }
}

pub async fn announcements(ctx: &Ctx, cmd: AnnouncementsCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        AnnouncementsCommand::List { cursor, limit } => {
            let page = client
                .announcements(&AnnouncementQuery {
                    cursor,
                    limit,
                    sticky: None,
                })
                .await?;
            ctx.print(&page, |p| {
                if p.items.is_empty() {
                    println!("no announcements");
                }
                for a in &p.items {
                    print_announcement(a, false);
                }
                if let Some(next) = &p.next_cursor {
                    println!("more: --cursor {next}");
                }
            })
        }
        AnnouncementsCommand::Show { id } => {
            let a = client.announcement(id).await?;
            ctx.print(&a, |a| print_announcement(a, true))
        }
        AnnouncementsCommand::Comments { id, cursor } => {
            let page = client
                .comments(
                    id,
                    &CommentQuery {
                        cursor,
                        limit: None,
                    },
                )
                .await?;
            ctx.print(&page, |p| {
                if p.items.is_empty() {
                    println!("no comments");
                }
                for c in &p.items {
                    print_comment(c);
                }
                if let Some(next) = &p.next_cursor {
                    println!("more: --cursor {next}");
                }
            })
        }
        AnnouncementsCommand::Comment { id, persona, body } => {
            let Some(text) = body.read()? else {
                bail!("give the comment with --body or --body-file");
            };
            let c = client
                .post_comment(
                    id,
                    &CreateComment {
                        persona: persona_id(&client, &persona).await?,
                        body: text,
                    },
                )
                .await?;
            ctx.print(&c, print_comment)
        }
        AnnouncementsCommand::DeleteComment { comment } => {
            client.delete_comment(comment).await?;
            ctx.done(&format!("deleted comment {comment}"))
        }
        AnnouncementsCommand::ReportComment {
            comment,
            reason,
            details,
        } => {
            client
                .report_comment(comment, &ReportComment { reason, details })
                .await?;
            ctx.done("report sent to the server's administrators")
        }
    }
}

fn print_comment(c: &Comment) {
    println!(
        "{}  {}  @{}{}",
        c.id,
        c.created_at.date(),
        c.author.handle,
        if c.removed {
            "  (removed by a server administrator)"
        } else {
            ""
        }
    );
    for line in c.body.lines() {
        println!("    {line}");
    }
}

pub async fn announce(ctx: &Ctx, cmd: AnnounceCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        AnnounceCommand::Post {
            title,
            body,
            no_comments,
            sticky,
        } => {
            let Some(text) = body.read()? else {
                bail!("give the text with --body or --body-file");
            };
            let a = client
                .admin_post_announcement(&CreateAnnouncement {
                    title,
                    body: text,
                    comments_open: !no_comments,
                    sticky,
                })
                .await?;
            ctx.print(&a, |a| print_announcement(a, false))
        }
        AnnounceCommand::Sticky { id, sticky } => {
            let a = client
                .admin_update_announcement(
                    id,
                    &UpdateAnnouncement {
                        sticky: Some(sticky),
                        ..UpdateAnnouncement::default()
                    },
                )
                .await?;
            ctx.print(&a, |a| print_announcement(a, false))
        }
        AnnounceCommand::Edit { id, title, body } => {
            let text = body.read()?;
            if title.is_none() && text.is_none() {
                bail!("give a new --title, or the text with --body or --body-file");
            }
            let a = client
                .admin_update_announcement(
                    id,
                    &UpdateAnnouncement {
                        title,
                        body: text,
                        ..UpdateAnnouncement::default()
                    },
                )
                .await?;
            ctx.print(&a, |a| print_announcement(a, false))
        }
        AnnounceCommand::RemoveComment { comment, reason } => {
            client
                .admin_remove_comment(comment, &RemoveComment { reason })
                .await?;
            ctx.done(&format!("removed comment {comment}"))
        }
        AnnounceCommand::Delete { id } => {
            client.admin_delete_announcement(id).await?;
            ctx.done(&format!("deleted announcement {id}"))
        }
        AnnounceCommand::Comments { id, open } => {
            let a = client
                .admin_update_announcement(
                    id,
                    &UpdateAnnouncement {
                        comments_open: Some(open),
                        ..UpdateAnnouncement::default()
                    },
                )
                .await?;
            ctx.print(&a, |a| print_announcement(a, false))
        }
    }
}
