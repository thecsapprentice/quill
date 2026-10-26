//! CLI commands for announcements: reading them (anyone) and posting,
//! editing and deleting them (administrators, under `quill admin announce`).

use anyhow::{Result, bail};
use clap::Subcommand;
use quill_proto::announcements::{
    Announcement, AnnouncementQuery, CreateAnnouncement, UpdateAnnouncement,
};
use uuid::Uuid;

use crate::{Ctx, board::BodyArgs};

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
        "{}  {}  {}{}",
        a.id,
        a.created_at.date(),
        a.title,
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
                .announcements(&AnnouncementQuery { cursor, limit })
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
    }
}

pub async fn announce(ctx: &Ctx, cmd: AnnounceCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        AnnounceCommand::Post {
            title,
            body,
            no_comments,
        } => {
            let Some(text) = body.read()? else {
                bail!("give the text with --body or --body-file");
            };
            let a = client
                .admin_post_announcement(&CreateAnnouncement {
                    title,
                    body: text,
                    comments_open: !no_comments,
                })
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
                        comments_open: None,
                    },
                )
                .await?;
            ctx.print(&a, |a| print_announcement(a, false))
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
