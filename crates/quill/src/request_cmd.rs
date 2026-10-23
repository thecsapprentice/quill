//! `quill request`: collaboration requests, both sides.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use quill_core::requests;
use quill_proto::requests::RequestStatus;
use serde_json::json;
use uuid::Uuid;

use crate::{Ctx, keys_cmd::own_persona};

#[derive(Subcommand)]
pub enum RequestCommand {
    /// Ask to write with a prompt's author. Only their devices can read
    /// the pitch.
    Send {
        prompt: Uuid,
        /// The persona asking (handle).
        #[arg(long = "as", value_name = "HANDLE")]
        persona: String,
        /// The pitch (Markdown).
        #[arg(long, conflicts_with = "pitch_file")]
        pitch: Option<String>,
        /// Read the pitch from a file.
        #[arg(long, value_name = "PATH")]
        pitch_file: Option<PathBuf>,
        /// Days to wait for an answer (1 to 30).
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
    /// This login's requests: waiting, session, or no response.
    List,
    /// Withdraw a request (or dismiss one that got no response).
    Withdraw { id: Uuid },
    /// Read the requests waiting on one of your prompts.
    Inbox {
        prompt: Uuid,
        /// Also release and read requests held while the prompt was busy.
        #[arg(long)]
        held: bool,
    },
    /// Accept a request you've read: it becomes a session.
    Accept { id: Uuid },
    /// Decline a request. The requester is told nothing.
    Decline { id: Uuid },
}

fn rfc3339(t: time::OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn status_word(s: RequestStatus) -> &'static str {
    match s {
        RequestStatus::Waiting => "waiting",
        RequestStatus::Session => "session",
        RequestStatus::NoResponse => "no response",
        RequestStatus::Pending => "pending",
        RequestStatus::Held => "held",
        RequestStatus::Declined => "declined",
        RequestStatus::Expired => "expired",
    }
}

pub async fn request(ctx: &Ctx, cmd: RequestCommand) -> Result<()> {
    match cmd {
        RequestCommand::Send {
            prompt,
            persona,
            pitch,
            pitch_file,
            days,
        } => {
            let text = match (pitch, pitch_file) {
                (Some(p), _) => p,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) => bail!("give a pitch with --pitch or --pitch-file"),
            };
            let persona = own_persona(ctx, &persona).await?;
            let client = ctx.client()?;
            let sent =
                requests::send(&ctx.store, &client, &persona, prompt, &text, Some(days)).await?;
            let body = json!({
                "id": sent.request.id,
                "expiresAt": rfc3339(sent.request.expires_at),
                "authorDevices": sent.author_devices,
                "ownDevices": sent.own_devices,
                "skipped": sent.skipped.iter().map(|(d, why)| json!({
                    "device": d, "reason": why,
                })).collect::<Vec<_>>(),
            });
            ctx.print(&body, |_| {
                println!(
                    "sent {} to {} of the author's devices; waiting until {}",
                    sent.request.id,
                    sent.author_devices,
                    sent.request.expires_at.date()
                );
                for (d, why) in &sent.skipped {
                    eprintln!("left out device {d}: {why}");
                }
            })
        }
        RequestCommand::List => {
            let list = requests::list(&ctx.client()?).await?;
            ctx.print(&list, |list| {
                if list.is_empty() {
                    println!("no requests");
                }
                for r in list {
                    let prompt = r
                        .prompt
                        .map_or_else(|| "deleted prompt".to_owned(), |p| p.to_string());
                    println!(
                        "{}  {}  {}  (until {})",
                        r.id,
                        status_word(r.status),
                        prompt,
                        r.expires_at.date()
                    );
                }
            })
        }
        RequestCommand::Withdraw { id } => {
            requests::withdraw(&ctx.store, &ctx.client()?, id).await?;
            ctx.done("withdrawn")
        }
        RequestCommand::Inbox { prompt, held } => {
            let inbox = requests::inbox(&ctx.store, &ctx.client()?, prompt, held).await?;
            let body = json!({
                "held": inbox.held,
                "requests": inbox.requests.iter().map(|r| json!({
                    "id": r.id,
                    "requester": r.requester,
                    "receivedAt": rfc3339(r.received_at),
                    "expiresAt": rfc3339(r.expires_at),
                    "pitch": r.pitch.as_ref().ok(),
                    "problem": r.pitch.as_ref().err(),
                })).collect::<Vec<_>>(),
            });
            ctx.print(&body, |_| {
                if inbox.requests.is_empty() {
                    println!("no pending requests");
                }
                for r in &inbox.requests {
                    println!(
                        "{}  from @{}  (until {})",
                        r.id,
                        r.requester.handle,
                        r.expires_at.date()
                    );
                    match &r.pitch {
                        Ok(text) => {
                            for line in text.lines() {
                                println!("    {line}");
                            }
                        }
                        Err(why) => println!("    (can't show it: {why})"),
                    }
                }
                if inbox.held > 0 {
                    println!(
                        "{} held while the prompt was busy: add --held to read them",
                        inbox.held
                    );
                }
            })
        }
        RequestCommand::Accept { id } => {
            let group = requests::accept(&ctx.store, &ctx.client()?, id).await?;
            ctx.print(&json!({ "groupId": group }), |_| {
                println!("accepted: the session's group is {group}");
            })
        }
        RequestCommand::Decline { id } => {
            requests::decline(&ctx.store, &ctx.client()?, id).await?;
            ctx.done("declined")
        }
    }
}
