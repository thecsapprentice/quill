//! `quill`: the minimal command-line client.
//!
//! Phase 0 covers the server address, accounts and personas. Each later phase
//! adds the commands that exercise its server features.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use quill_core::{Client, Profile};
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
    /// Profile file (default: platform config dir, or $QUILL_PROFILE).
    #[arg(long, global = true)]
    profile: Option<PathBuf>,

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

struct Ctx {
    path: PathBuf,
    profile: Profile,
    json: bool,
}

impl Ctx {
    fn client(&self) -> Result<Client> {
        let server = self
            .profile
            .server
            .as_deref()
            .context("no server set; run `quill server <url>` first")?;
        let client = Client::new(server)?;
        Ok(match &self.profile.token {
            Some(token) => client.with_token(token.as_str()),
            None => client,
        })
    }

    fn save(&self) -> Result<()> {
        self.profile.save(&self.path)?;
        Ok(())
    }

    fn print<T: Serialize>(&self, value: &T, human: impl FnOnce(&T)) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string_pretty(value)?);
        } else {
            human(value);
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
    let path = match cli.profile {
        Some(p) => p,
        None => Profile::default_path()?,
    };
    let profile = Profile::load(&path)?;
    let mut ctx = Ctx {
        path,
        profile,
        json: cli.json,
    };

    match cli.command {
        Command::Server { url: None } => {
            let server = ctx.profile.server.clone().unwrap_or_default();
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
            if ctx.profile.server.as_deref() != Some(url.as_str()) {
                // A token is only valid on the server that issued it.
                ctx.profile.token = None;
            }
            ctx.profile.server = Some(url);
            ctx.save()?;
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
            ctx.profile.token = Some(Zeroizing::new(token.token));
            ctx.save()?;
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
            ctx.profile.token = Some(Zeroizing::new(token.token));
            ctx.save()?;
            if !ctx.json {
                println!("logged in");
            }
        }
        Command::Logout => {
            if ctx.profile.token.is_some() {
                // Forget the token locally even if the server is unreachable.
                let result = ctx.client()?.logout().await;
                ctx.profile.token = None;
                ctx.save()?;
                result?;
            }
            if !ctx.json {
                println!("logged out");
            }
        }
        Command::Persona(cmd) => persona(&ctx, cmd).await?,
    }
    Ok(())
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
