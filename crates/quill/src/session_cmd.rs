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
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
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
        /// Attach a file (any kind; repeat for more).
        #[arg(long = "attach", value_name = "FILE")]
        attach: Vec<PathBuf>,
    },
    /// Show the safety numbers to compare with the other writer, in person
    /// or over another channel you trust.
    Verify {
        #[arg(allow_hyphen_values = true)]
        id: String,
        /// They match: mark the other writer verified on this device.
        #[arg(long)]
        confirm: bool,
    },
    /// Save a message's attachments into a folder.
    Save {
        #[arg(allow_hyphen_values = true)]
        id: String,
        /// The message's number, as `read` shows it.
        message: u64,
        #[arg(long, value_name = "DIR")]
        to: PathBuf,
    },
    /// A session's channels: what each is for, and what you haven't seen.
    Channels {
        #[arg(allow_hyphen_values = true)]
        id: String,
    },
    /// Name a channel, or describe it again.
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// Change one of your messages. Every version is kept.
    Edit {
        #[arg(allow_hyphen_values = true)]
        id: String,
        /// The message's number, as `read` shows it.
        message: u64,
        #[arg(long)]
        text: String,
    },
    /// Delete one of your messages. It shows as deleted; its versions are
    /// kept.
    Delete {
        #[arg(allow_hyphen_values = true)]
        id: String,
        message: u64,
    },
    /// Every version of a message.
    History {
        #[arg(allow_hyphen_values = true)]
        id: String,
        message: u64,
    },
    /// Leave a session. With two writers, that ends it for both; what's
    /// been written stays readable.
    Leave {
        #[arg(allow_hyphen_values = true)]
        id: String,
    },
}

fn standing_text(s: &Standing) -> String {
    match s {
        Standing::Joined => "joined".into(),
        Standing::Waiting => "waiting for another of your devices to add this one".into(),
        Standing::NoKeys => "no keys for this persona on this device".into(),
        Standing::Rejoining => {
            "away too long: waiting to be let back in by another of your devices or the other writer"
                .into()
        }
        Standing::Broken(why) => format!("stopped: {why}"),
    }
}

#[derive(Subcommand)]
pub enum ChannelCommand {
    /// Name a channel so it shows for both writers, or describe it again.
    Add {
        #[arg(allow_hyphen_values = true)]
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
        // Sent by this login's persona, from this device or another.
        let mine = reading
            .session
            .members
            .iter()
            .any(|x| x.persona.is_some() && x.actor_uri == m.sender);
        let who = if m.own || mine {
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
        for a in &m.attachments {
            let have = matches!(sessions::file(store, &a.url), Ok(Some(_)));
            let note = if have {
                ""
            } else {
                ", not available on this device"
            };
            println!(
                "    [attachment] {} ({} bytes, {}{note})",
                a.name, a.size, a.media_type
            );
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
                "historyIncomplete": l.history_missing.is_some(),
                "verified": sessions::verified(&ctx.store, &l.session).unwrap_or(false),
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
            let mark = if sessions::verified(&ctx.store, &l.session).unwrap_or(false) {
                " (verified)"
            } else {
                ""
            };
            println!(
                "{}  @{me} with {partner}{mark}  {}{}  {} messages{}",
                l.session.id,
                if l.session.ended { "ended, " } else { "" },
                standing_text(&l.standing),
                l.messages,
                if l.history_missing.is_some() {
                    " (earlier history still to come from your other devices)"
                } else {
                    ""
                }
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
            if reading.history_missing.is_some() {
                // Until it arrives, numbers missing in between are expected.
                eprintln!(
                    "note: this device doesn't have all of the session's earlier history yet; your other devices send it when they next run"
                );
            } else {
                for g in &reading.gaps {
                    eprintln!(
                        "warning: {} message(s) from one device never arrived; the server may be holding them back",
                        g.missing.len()
                    );
                }
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
            attach,
        } => {
            let text = match (text, file) {
                (Some(t), _) => t,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) if !attach.is_empty() => String::new(),
                (None, None) => bail!("give the message with --text or --file"),
            };
            let mut files = Vec::new();
            for path in &attach {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "attachment".into());
                let contents =
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
                files.push((name, contents));
            }
            let seq = sessions::post(
                &ctx.store,
                &client,
                &id,
                sessions::Draft {
                    text: &text,
                    channel: channel.as_deref(),
                    reply_to,
                    files,
                },
            )
            .await?;
            ctx.print(&json!({ "seq": seq }), |_| println!("sent (#{seq})"))
        }
        SessionCommand::Save { id, message, to } => {
            let m = sessions::versions(&ctx.store, &client, &id, message).await?;
            if m.attachments.is_empty() {
                bail!("message #{message} has no attachments");
            }
            std::fs::create_dir_all(&to).with_context(|| format!("creating {}", to.display()))?;
            let mut saved = Vec::new();
            for a in &m.attachments {
                let Some(contents) = sessions::file(&ctx.store, &a.url)? else {
                    eprintln!("{}: not available on this device", a.name);
                    continue;
                };
                // Never overwrite: add a number until the name is free.
                let name = quill_core::attachments::safe_name(&a.name);
                let mut path = to.join(&name);
                let mut n = 1;
                while path.exists() {
                    path = to.join(format!("{n}-{name}"));
                    n += 1;
                }
                std::fs::write(&path, contents.as_slice())
                    .with_context(|| format!("writing {}", path.display()))?;
                saved.push(path.display().to_string());
            }
            ctx.print(&saved, |saved| {
                for p in saved {
                    println!("saved {p}");
                }
            })
        }
        SessionCommand::Verify { id, confirm } => {
            let n = if confirm {
                sessions::confirm_verified(&ctx.store, &client, &id).await?
            } else {
                sessions::safety_numbers(&ctx.store, &client, &id).await?
            };
            let body = json!({
                "session": n.session,
                "partner": n.partner.handle,
                "identity": n.identity.join(" "),
                "epoch": n.epoch,
                "check": n.check.join(" "),
                "verified": n.verified,
            });
            ctx.print(&body, |_| {
                println!(
                    "Compare these with @{}, in person or over a channel you trust:",
                    n.partner.handle
                );
                println!();
                println!("  identity number:  {}", n.identity[..6].join(" "));
                println!("                    {}", n.identity[6..].join(" "));
                println!(
                    "  session check:    {} (epoch {})",
                    n.check.join(" "),
                    n.epoch
                );
                println!();
                println!("The identity number changes only if either of your identity keys does.");
                println!("The session check matches only between devices at the same epoch.");
                if n.verified {
                    println!("@{} is verified on this device.", n.partner.handle);
                } else if !confirm {
                    println!("If they match, run this again with --confirm.");
                }
            })
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: SessionCommand,
    }

    fn parse(args: &[&str]) -> SessionCommand {
        let argv = std::iter::once("session").chain(args.iter().copied());
        Cli::try_parse_from(argv)
            .unwrap_or_else(|e| panic!("{args:?}: {e}"))
            .command
    }

    // Session ids are base64url, so about one in 64 starts with a hyphen.
    #[test]
    fn session_ids_may_start_with_a_hyphen() {
        let SessionCommand::Write { id, text, .. } = parse(&["write", "-Lx9", "--text", "hi"])
        else {
            panic!("not write");
        };
        assert_eq!((id.as_str(), text.as_deref()), ("-Lx9", Some("hi")));
        let SessionCommand::Read { id, follow, .. } = parse(&["read", "--follow", "-Lx9"]) else {
            panic!("not read");
        };
        assert_eq!((id.as_str(), follow), ("-Lx9", true));
        let SessionCommand::Delete { id, message } = parse(&["delete", "--", "--x9", "3"]) else {
            panic!("not delete");
        };
        assert_eq!((id.as_str(), message), ("--x9", 3));
        let SessionCommand::Read { id, channel, .. } = parse(&["read", "abc", "--channel", "ooc"])
        else {
            panic!("not read");
        };
        assert_eq!((id.as_str(), channel.as_deref()), ("abc", Some("ooc")));
    }
}
