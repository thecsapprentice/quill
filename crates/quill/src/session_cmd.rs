//! `quill sessions` and `quill session`: writing together.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use quill_core::sessions::{self, Standing};
use serde_json::json;

use crate::Ctx;

#[derive(Subcommand)]
pub enum SessionCommand {
    /// Show a session's messages.
    Read {
        /// The session id, or enough of its start to be unambiguous.
        id: String,
        /// Only this channel.
        #[arg(long)]
        channel: Option<String>,
    },
    /// Write to a session.
    Write {
        id: String,
        /// The message (Markdown).
        #[arg(long, conflicts_with = "file")]
        text: Option<String>,
        /// Read the message from a file.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// The channel (default: story).
        #[arg(long)]
        channel: Option<String>,
    },
    /// Leave a session. With two writers, that ends it for both; what's
    /// been written stays readable.
    Leave { id: String },
}

fn standing_text(s: &Standing) -> String {
    match s {
        Standing::Joined => "joined".into(),
        Standing::Waiting => "waiting for another of your devices to add this one".into(),
        Standing::NoKeys => "no keys for this persona on this device".into(),
        Standing::Broken(why) => format!("stopped: {why}"),
    }
}

pub async fn list(ctx: &Ctx) -> Result<()> {
    let listed = sessions::list(&ctx.store, &ctx.client()?).await?;
    let body: Vec<_> = listed
        .iter()
        .map(|l| {
            json!({
                "id": l.session.id,
                "request": l.session.request,
                "prompt": l.session.prompt,
                "me": l.me().map(|m| &m.handle),
                "partner": l.partner().map(|m| &m.handle),
                "ended": l.session.ended,
                "standing": standing_text(&l.standing),
                "messages": l.messages,
            })
        })
        .collect();
    ctx.print(&body, |_| {
        if listed.is_empty() {
            println!("no sessions");
        }
        for l in &listed {
            let partner = l
                .partner()
                .map_or_else(|| "(gone)".to_owned(), |m| format!("@{}", m.handle));
            let me = l.me().map_or("?", |m| m.handle.as_str());
            println!(
                "{}  @{me} with {partner}  {}{}  {} messages",
                l.session.id,
                if l.session.ended { "ended, " } else { "" },
                standing_text(&l.standing),
                l.messages
            );
        }
    })
}

pub async fn session(ctx: &Ctx, cmd: SessionCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        SessionCommand::Read { id, channel } => {
            let found = sessions::find(&client, &id).await?;
            let messages = sessions::read(&ctx.store, &client, &found.id).await?;
            let messages: Vec<_> = messages
                .into_iter()
                .filter(|m| channel.as_ref().is_none_or(|c| *c == m.channel))
                .collect();
            ctx.print(&messages, |messages| {
                for m in messages {
                    let who = if m.own {
                        "you".to_owned()
                    } else {
                        sessions::sender_name(&found, &m.sender)
                    };
                    println!("#{} [{}] {who}:", m.seq, m.channel);
                    for line in m.content.lines() {
                        println!("    {line}");
                    }
                }
            })
        }
        SessionCommand::Write {
            id,
            text,
            file,
            channel,
        } => {
            let text = match (text, file) {
                (Some(t), _) => t,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) => bail!("give the message with --text or --file"),
            };
            let seq = sessions::write(&ctx.store, &client, &id, &text, channel.as_deref()).await?;
            ctx.print(&json!({ "seq": seq }), |_| println!("sent (#{seq})"))
        }
        SessionCommand::Leave { id } => {
            sessions::leave(&ctx.store, &client, &id).await?;
            ctx.done("left: the session has ended")
        }
    }
}
