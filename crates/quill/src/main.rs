//! `quill`: the minimal command-line client.
//!
//! Covers the server address, accounts, personas and the prompt board. Each
//! later phase adds the commands that exercise its server features.

mod board;
mod keys_cmd;
mod local;
mod request_cmd;
mod session_cmd;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use quill_core::{Client, Profile, Store};
use quill_proto::api::{CreatePersona, Credentials, UpdatePersona};
use serde::Serialize;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Environment variable read for the password in non-interactive use (tests,
/// scripts). Prefer the interactive prompt.
const PASSWORD_ENV: &str = "QUILL_PASSWORD";

#[derive(Parser)]
#[command(name = "quill", version, about = "Quill command-line client")]
struct Cli {
    /// Encrypted local store (default: platform data dir, or $QUILL_STORE).
    #[arg(long, global = true)]
    store: Option<PathBuf>,

    /// Print results as JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set or show the server this profile talks to.
    Server {
        /// Server base URL, e.g. https://writers.example
        url: Option<String>,
    },
    /// Show the server's name, versions and limits.
    Instance,
    /// Create an account and log in.
    Register(UsernameArg),
    /// Log in to an existing account.
    Login(UsernameArg),
    /// Log out and forget the token.
    Logout,
    /// Manage personas (public identities).
    #[command(subcommand)]
    Persona(PersonaCommand),
    /// Show or change account settings.
    #[command(subcommand)]
    Account(board::AccountCommand),
    /// Post, search and manage prompts.
    #[command(subcommand)]
    Prompt(board::PromptCommand),
    /// Browse tags and tag namespaces.
    #[command(subcommand)]
    Tags(board::TagCommand),
    /// Report a prompt to the server's administrators.
    Report {
        prompt: Uuid,
        /// illegal, spam, harassment, wrong-maturity or other
        #[arg(long)]
        reason: quill_proto::board::ReportReason,
        #[arg(long)]
        details: Option<String>,
    },
    /// Administrator tools: reports, the admin log and tags.
    #[command(subcommand)]
    Admin(board::AdminCommand),
    /// The encrypted local store holding this client's login and keys.
    #[command(subcommand)]
    Store(local::StoreCommand),
    /// Persona identity keys: create, publish, and check anyone's.
    #[command(subcommand)]
    Keys(keys_cmd::KeysCommand),
    /// Devices: link a new computer, approve one, list a persona's devices,
    /// revoke or remove.
    #[command(subcommand)]
    Device(keys_cmd::DeviceCommand),
    /// This login's devices and their persona devices.
    Devices,
    /// Collaboration requests: send one, read your inbox, accept or decline.
    #[command(subcommand)]
    Request(request_cmd::RequestCommand),
    /// This login's sessions, and how this device stands in each.
    Sessions,
    /// Read, write to, or leave a session.
    #[command(subcommand)]
    Session(session_cmd::SessionCommand),
}

#[derive(Args)]
struct UsernameArg {
    /// Private login name (never shown publicly).
    username: String,
}

#[derive(Subcommand)]
enum PersonaCommand {
    /// List this account's personas.
    List,
    /// Create a persona.
    New {
        handle: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        summary: Option<String>,
    },
    /// Change a persona's display name or summary (empty string clears it).
    Edit {
        id: Uuid,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        summary: Option<String>,
    },
    /// Delete a persona.
    Delete { id: Uuid },
}

pub struct Ctx {
    store: Store,
    json: bool,
}

impl Ctx {
    fn client(&self) -> Result<Client> {
        let server = self
            .store
            .server()?
            .context("no server set; run `quill server <url>` first")?;
        let client = Client::new(&server)?;
        Ok(match self.store.token()? {
            Some(token) => client.with_token(token.as_str()),
            None => client,
        })
    }

    fn print<T: Serialize>(&self, value: &T, human: impl FnOnce(&T)) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string_pretty(value)?);
        } else {
            human(value);
        }
        Ok(())
    }

    /// Reports success for commands that return nothing.
    fn done(&self, message: &str) -> Result<()> {
        if self.json {
            println!("{}", serde_json::json!({ "ok": true }));
        } else {
            println!("{message}");
        }
        Ok(())
    }
}

fn read_password(confirm: bool) -> Result<Zeroizing<String>> {
    if let Ok(p) = std::env::var(PASSWORD_ENV) {
        return Ok(Zeroizing::new(p));
    }
    let first = Zeroizing::new(rpassword::prompt_password("Password: ")?);
    if confirm {
        let second = Zeroizing::new(rpassword::prompt_password("Repeat password: ")?);
        if *first != *second {
            bail!("passwords do not match");
        }
    }
    Ok(first)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let path = match cli.store {
        Some(p) => p,
        None => quill_core::store::default_path()?,
    };
    // Earlier versions kept the server and token in a plain profile file;
    // a new store takes them over.
    let legacy = Profile::default_path().ok();
    let store = local::open(&path, legacy)?;
    let mut ctx = Ctx {
        store,
        json: cli.json,
    };

    if needs_upkeep(&cli.command) {
        upkeep(&ctx).await;
    }

    match cli.command {
        Command::Server { url: None } => {
            let server = ctx.store.server()?.unwrap_or_default();
            ctx.print(&server, |s| {
                if s.is_empty() {
                    println!("no server set");
                } else {
                    println!("{s}");
                }
            })?;
        }
        Command::Server { url: Some(url) } => {
            quill_core::client::check_server_url(&url)?;
            // A different server also forgets the token: only the server
            // that issued it accepts it.
            ctx.store.set_server(&url)?;
        }
        Command::Instance => {
            let info = ctx.client()?.instance().await?;
            ctx.print(&info, |i| {
                println!("{} ({} {})", i.domain, i.software, i.software_version);
                println!("protocol versions: {}", i.protocol_versions.join(", "));
                println!("registration open: {}", i.registration_open);
                println!("max message bytes: {}", i.limits.max_message_bytes);
                println!("max attachment bytes: {}", i.limits.max_attachment_bytes);
            })?;
        }
        Command::Register(UsernameArg { username }) => {
            let password = read_password(true)?;
            let creds = Credentials {
                username,
                password: password.as_str().to_owned(),
            };
            let token = ctx.client()?.register(&creds).await?;
            ctx.store.set_token(Some(&token.token))?;
            if !ctx.json {
                println!("registered and logged in");
            }
        }
        Command::Login(UsernameArg { username }) => {
            let password = read_password(false)?;
            let creds = Credentials {
                username,
                password: password.as_str().to_owned(),
            };
            let token = ctx.client()?.login(&creds).await?;
            ctx.store.set_token(Some(&token.token))?;
            if !ctx.json {
                println!("logged in");
            }
        }
        Command::Logout => {
            if ctx.store.token()?.is_some() {
                // Forget the token locally even if the server is unreachable.
                let result = ctx.client()?.logout().await;
                ctx.store.set_token(None)?;
                result?;
            }
            if !ctx.json {
                println!("logged out");
            }
        }
        Command::Persona(cmd) => persona(&ctx, cmd).await?,
        Command::Account(cmd) => board::account(&ctx, cmd).await?,
        Command::Prompt(cmd) => board::prompt(&ctx, cmd).await?,
        Command::Tags(cmd) => board::tags(&ctx, cmd).await?,
        Command::Report {
            prompt,
            reason,
            details,
        } => board::report(&ctx, prompt, reason, details).await?,
        Command::Admin(cmd) => board::admin(&ctx, cmd).await?,
        Command::Store(cmd) => local::command(&mut ctx, cmd)?,
        Command::Keys(cmd) => keys_cmd::keys(&ctx, cmd).await?,
        Command::Device(cmd) => keys_cmd::device(&ctx, cmd).await?,
        Command::Devices => keys_cmd::devices(&ctx).await?,
        Command::Request(cmd) => request_cmd::request(&ctx, cmd).await?,
        Command::Sessions => session_cmd::list(&ctx).await?,
        Command::Session(cmd) => session_cmd::session(&ctx, cmd).await?,
    }
    Ok(())
}

/// Commands that act as a signed-in writer get upkeep first.
fn needs_upkeep(command: &Command) -> bool {
    !matches!(
        command,
        Command::Server { .. }
            | Command::Instance
            | Command::Register(_)
            | Command::Login(_)
            | Command::Logout
            | Command::Store(_)
            | Command::Device(keys_cmd::DeviceCommand::Link { .. })
    )
}

/// Catches up with this login's other devices and keeps KeyPackage pools
/// full. Problems are reported but don't stop the command.
async fn upkeep(ctx: &Ctx) {
    if !matches!(ctx.store.token(), Ok(Some(_))) {
        return;
    }
    let Ok(client) = ctx.client() else {
        return;
    };
    match quill_core::devices::maintain(&ctx.store, &client).await {
        Ok(report) if report.caught_up.removed => eprintln!(
            "This device was removed from your login's devices: its persona keys are gone. Link it again with `quill device link`."
        ),
        Ok(report) => {
            for (id, why) in &report.sessions.problems {
                eprintln!("warning: couldn't sync session {id}: {why}");
            }
        }
        Err(e) => eprintln!("warning: couldn't sync with your other devices: {e}"),
    }
}

async fn persona(ctx: &Ctx, cmd: PersonaCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        PersonaCommand::List => {
            let list = client.personas().await?;
            ctx.print(&list, |l| {
                if l.is_empty() {
                    println!("no personas");
                }
                for p in l {
                    println!(
                        "{}  @{}  {}",
                        p.id,
                        p.handle,
                        p.display_name.as_deref().unwrap_or("")
                    );
                }
            })?;
        }
        PersonaCommand::New {
            handle,
            display_name,
            summary,
        } => {
            let p = client
                .create_persona(&CreatePersona {
                    handle,
                    display_name,
                    summary,
                })
                .await?;
            ctx.print(&p, |p| println!("created @{} ({})", p.handle, p.id))?;
        }
        PersonaCommand::Edit {
            id,
            display_name,
            summary,
        } => {
            let p = client
                .update_persona(
                    id,
                    &UpdatePersona {
                        display_name,
                        summary,
                    },
                )
                .await?;
            ctx.print(&p, |p| println!("updated @{}", p.handle))?;
        }
        PersonaCommand::Delete { id } => {
            client.delete_persona(id).await?;
            if !ctx.json {
                println!("deleted {id}");
            }
        }
    }
    Ok(())
}
