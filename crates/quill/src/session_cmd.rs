//! `quill sessions` and `quill session`: writing together.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use quill_core::{
    Store,
    sessions::{self, Message, Reading, Standing},
};
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
        /// Stay open, printing new and changed messages as they arrive.
        #[arg(long)]
        follow: bool,
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
        /// The number of the message this answers, in any channel.
        #[arg(long, value_name = "N")]
        reply_to: Option<u64>,
    },
    /// A session's channels: what each is for, and what you haven't seen.
    Channels { id: String },
    /// Name a channel, or describe it again.
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// Change one of your messages. Every version is kept.
    Edit {
        id: String,
        /// The message's number, as `read` shows it.
        message: u64,
        #[arg(long)]
        text: String,
    },
    /// Delete one of your messages. It shows as deleted; its versions are
    /// kept.
    Delete { id: String, message: u64 },
    /// Every version of a message.
    History { id: String, message: u64 },
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

#[derive(Subcommand)]
pub enum ChannelCommand {
    /// Name a channel so it shows for both writers, or describe it again.
    Add {
        id: String,
        /// Lowercase letters, digits and hyphens; at most 32.
        name: String,
        /// What the channel is for.
        #[arg(long)]
        about: Option<String>,
    },
}

fn print_messages(store: &Store, reading: &Reading, messages: &[&Message]) {
    for m in messages {
        let who = if m.own {
            "you".to_owned()
        } else {
            sessions::sender_name(&reading.session, &m.sender)
        };
        let mark = if m.deleted {
            " (deleted)"
        } else if m.edited() {
            " (edited)"
        } else {
            ""
        };
        println!("#{} [{}] {who}{mark}:", m.seq, m.channel);
        if let Some(to) = &m.in_reply_to {
            match sessions::locate(store, &reading.session, to) {
                Ok(Some((seq, channel))) => println!("    (answers #{seq} in [{channel}])"),
                _ => println!("    (answers a message this device doesn't have)"),
            }
        }
        if m.deleted {
            println!("    deleted by author");
            continue;
        }
        for line in m.content.lines() {
            println!("    {line}");
        }
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
        SessionCommand::Read {
            id,
            channel,
            follow,
        } => {
            let wanted = |m: &&Message| channel.as_ref().is_none_or(|c| *c == m.channel);
            if follow {
                let json = ctx.json;
                return Ok(
                    sessions::follow(&ctx.store, &client, &id, |reading, fresh| {
                        let fresh: Vec<&Message> = fresh.iter().filter(wanted).collect();
                        if json {
                            for m in fresh {
                                println!("{}", serde_json::to_string(m).unwrap_or_default());
                            }
                        } else {
                            print_messages(&ctx.store, reading, &fresh);
                        }
                        true
                    })
                    .await?,
                );
            }
            let reading = sessions::read(&ctx.store, &client, &id).await?;
            let messages: Vec<&Message> = reading.messages.iter().filter(wanted).collect();
            ctx.print(&messages, |messages| {
                print_messages(&ctx.store, &reading, messages)
            })?;
            sessions::mark_shown(&ctx.store, &reading, channel.as_deref())?;
            for g in &reading.gaps {
                eprintln!(
                    "warning: {} message(s) from one device never arrived; the server may be holding them back",
                    g.missing.len()
                );
            }
            Ok(())
        }
        SessionCommand::Edit { id, message, text } => {
            let seq = sessions::edit(&ctx.store, &client, &id, message, &text).await?;
            ctx.print(&json!({ "seq": seq }), |_| println!("edited #{message}"))
        }
        SessionCommand::Delete { id, message } => {
            let seq = sessions::delete(&ctx.store, &client, &id, message).await?;
            ctx.print(&json!({ "seq": seq }), |_| println!("deleted #{message}"))
        }
        SessionCommand::History { id, message } => {
            let m = sessions::versions(&ctx.store, &client, &id, message).await?;
            ctx.print(&m, |m| {
                for (i, v) in m.versions.iter().enumerate() {
                    println!("version {} (#{}, {}):", i + 1, v.seq, v.hlc);
                    match &v.content {
                        Some(text) => {
                            for line in text.lines() {
                                println!("    {line}");
                            }
                        }
                        None => println!("    deleted"),
                    }
                }
            })
        }
        SessionCommand::Write {
            id,
            text,
            file,
            channel,
            reply_to,
        } => {
            let text = match (text, file) {
                (Some(t), _) => t,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) => bail!("give the message with --text or --file"),
            };
            let seq = match reply_to {
                Some(to) => {
                    sessions::reply(&ctx.store, &client, &id, &text, channel.as_deref(), to).await?
                }
                None => {
                    sessions::write(&ctx.store, &client, &id, &text, channel.as_deref()).await?
                }
            };
            ctx.print(&json!({ "seq": seq }), |_| println!("sent (#{seq})"))
        }
        SessionCommand::Channels { id } => {
            let list = sessions::channels(&ctx.store, &client, &id).await?;
            ctx.print(&list, |list| {
                for c in list {
                    let about = c
                        .summary
                        .as_deref()
                        .map(|s| format!("  {s}"))
                        .unwrap_or_default();
                    println!("{}  {} notes, {} new{about}", c.name, c.notes, c.unseen);
                }
            })
        }
        SessionCommand::Channel(ChannelCommand::Add { id, name, about }) => {
            sessions::name_channel(&ctx.store, &client, &id, &name, about.as_deref()).await?;
            ctx.done(&format!("channel {name} is ready"))
        }
        SessionCommand::Leave { id } => {
            sessions::leave(&ctx.store, &client, &id).await?;
            ctx.done("left: the session has ended")
        }
    }
}
