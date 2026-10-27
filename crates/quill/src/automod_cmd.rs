//! `quill admin automod`: moderation scripts, their chains, common data,
//! templates and dry runs. Administrators only.

use std::io::Read;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use quill_proto::automod::{
    DataOverride, DryRun, DryRunDecision, Hook, ModerationScript, OnError, ReplaceScript,
    SaveModerationData, SetScriptMode, UpdateChain, UploadScript, UseTemplate,
};
use uuid::Uuid;

use crate::Ctx;

#[derive(Subcommand)]
pub enum AutomodCommand {
    /// List scripts: whether each is on, log-only, and its chains.
    Scripts,
    /// Show a script's source and what it reads (by id or name).
    Show { script: String },
    /// Upload a script from a file (`-` for standard input).
    Upload {
        name: String,
        file: String,
        /// Start in log-only mode: results are logged, nothing is done.
        #[arg(long)]
        log_only: bool,
    },
    /// Replace a script's source from a file, or rename it.
    Replace {
        script: String,
        /// The new source (`-` for standard input).
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Switch a script on or off, or into or out of log-only mode.
    Mode {
        script: String,
        /// on or off.
        #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
        enabled: Option<bool>,
        /// on or off.
        #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
        log_only: Option<bool>,
    },
    /// Remove a script, and its place in every chain.
    Remove { script: String },
    /// Show every hook's chain.
    Chains,
    /// Set a hook's chain: its scripts in order (ids or names), and what a
    /// failing script means.
    Chain {
        hook: Hook,
        /// continue, reject or flag.
        #[arg(long)]
        on_error: OnError,
        scripts: Vec<String>,
    },
    /// List the templates scripts can be started from.
    Templates,
    /// Start a new script from a template.
    Template {
        id: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Run a chain, or one script, without doing anything.
    DryRun {
        hook: Hook,
        /// Just this script (id or name), not the whole chain.
        #[arg(long)]
        script: Option<String>,
        /// Run on this existing prompt.
        #[arg(long, conflicts_with_all = ["title", "body"])]
        prompt: Option<Uuid>,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value = "")]
        body: String,
        /// Try unsaved common data: name=file.
        #[arg(long = "data", value_name = "NAME=FILE")]
        data: Vec<String>,
    },
    /// Common data that scripts read.
    #[command(subcommand)]
    Data(DataCommand),
}

#[derive(Subcommand)]
pub enum DataCommand {
    /// List entries, their sizes and the scripts reading them.
    List,
    /// Print an entry's content.
    Get { name: String },
    /// Create an entry, or save a new version, from a file (`-` for
    /// standard input).
    Set {
        name: String,
        file: String,
        #[arg(long)]
        description: Option<String>,
        /// Refuse if someone has saved since this version.
        #[arg(long)]
        base: Option<i32>,
    },
    /// Delete an entry and its history (refused while a script reads it).
    Remove { name: String },
    /// List an entry's saved versions.
    History { name: String },
    /// Print one saved version.
    Version { name: String, version: i32 },
    /// Undo: save an old version again, as a new version.
    Restore { name: String, version: i32 },
}

fn read_file(path: &str) -> Result<String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {path}"))
}

/// A script by id, or by name (any case).
async fn script_id(client: &quill_core::Client, script: &str) -> Result<Uuid> {
    if let Ok(id) = script.parse() {
        return Ok(id);
    }
    let all = client.automod_scripts().await?;
    match all.iter().find(|s| s.name.eq_ignore_ascii_case(script)) {
        Some(s) => Ok(s.id),
        None => bail!("no script named {script:?}"),
    }
}

fn mode(s: &ModerationScript) -> &'static str {
    match (s.enabled, s.log_only) {
        (false, _) => "off",
        (true, true) => "log only",
        (true, false) => "on",
    }
}

pub async fn automod(ctx: &Ctx, cmd: AutomodCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        AutomodCommand::Scripts => {
            let scripts = client.automod_scripts().await?;
            if scripts.is_empty() {
                println!(
                    "No scripts yet. `quill admin automod templates` lists ones to start from."
                );
            }
            for s in scripts {
                let chains: Vec<&str> = s.chains.iter().map(|h| h.as_str()).collect();
                let chains = if chains.is_empty() {
                    "in no chain".to_owned()
                } else {
                    chains.join(", ")
                };
                println!("{}  {}  [{}]  {}", s.id, s.name, mode(&s), chains);
            }
        }
        AutomodCommand::Show { script } => {
            let s = client
                .automod_script(script_id(&client, &script).await?)
                .await?;
            println!("{} ({}, {})", s.name, s.id, mode(&s));
            if !s.data.is_empty() {
                println!("Reads: {}", s.data.join(", "));
            }
            println!("Source hash: {}\n", s.source_hash);
            println!("{}", s.source);
        }
        AutomodCommand::Upload {
            name,
            file,
            log_only,
        } => {
            let source = read_file(&file)?;
            let s = client
                .automod_upload(&UploadScript {
                    name,
                    source,
                    log_only,
                })
                .await?;
            println!(
                "Uploaded {} ({}, {}). Add it to a chain with `quill admin automod chain`.",
                s.name,
                s.id,
                mode(&s)
            );
        }
        AutomodCommand::Replace { script, file, name } => {
            if file.is_none() && name.is_none() {
                bail!("give --file, --name or both");
            }
            let source = file.as_deref().map(read_file).transpose()?;
            let id = script_id(&client, &script).await?;
            let s = client
                .automod_replace(id, &ReplaceScript { name, source })
                .await?;
            println!("Saved {}.", s.name);
        }
        AutomodCommand::Mode {
            script,
            enabled,
            log_only,
        } => {
            let id = script_id(&client, &script).await?;
            let s = client
                .automod_set_mode(id, &SetScriptMode { enabled, log_only })
                .await?;
            println!("{} is {}.", s.name, mode(&s));
        }
        AutomodCommand::Remove { script } => {
            client
                .automod_remove(script_id(&client, &script).await?)
                .await?;
            println!("Removed.");
        }
        AutomodCommand::Chains => {
            for c in client.automod_chains().await? {
                let names: Vec<String> = c
                    .steps
                    .iter()
                    .map(|s| match (s.enabled, s.log_only) {
                        (false, _) => format!("{} (off)", s.name),
                        (true, true) => format!("{} (log only)", s.name),
                        (true, false) => s.name.clone(),
                    })
                    .collect();
                let steps = if names.is_empty() {
                    "nothing".to_owned()
                } else {
                    names.join(" → ")
                };
                println!(
                    "{:<16} on error: {:<8} {steps}",
                    c.hook.as_str(),
                    c.on_error.as_str()
                );
            }
        }
        AutomodCommand::Chain {
            hook,
            on_error,
            scripts,
        } => {
            let mut ids = Vec::new();
            for s in &scripts {
                ids.push(script_id(&client, s).await?);
            }
            let c = client
                .automod_update_chain(
                    hook,
                    &UpdateChain {
                        on_error,
                        scripts: ids,
                    },
                )
                .await?;
            println!(
                "{}: {} script(s), on error {}.",
                c.hook,
                c.steps.len(),
                c.on_error
            );
        }
        AutomodCommand::Templates => {
            for t in client.automod_templates().await? {
                println!("{:<16} {}: {}", t.id, t.name, t.summary);
            }
        }
        AutomodCommand::Template { id, name } => {
            let s = client
                .automod_use_template(&id, &UseTemplate { name })
                .await?;
            println!("Created {} ({}).", s.name, s.id);
            for d in &s.data {
                println!(
                    "It reads the common data entry {d}: fill it with `quill admin automod data set {d} <file>`."
                );
            }
        }
        AutomodCommand::DryRun {
            hook,
            script,
            prompt,
            title,
            body,
            data,
        } => {
            let script = match script {
                Some(s) => Some(script_id(&client, &s).await?),
                None => None,
            };
            let mut overrides = Vec::new();
            for d in data {
                let Some((name, file)) = d.split_once('=') else {
                    bail!("--data takes name=file, like blocked_words=words.txt");
                };
                overrides.push(DataOverride {
                    name: name.to_owned(),
                    content: read_file(file)?,
                });
            }
            let result = client
                .automod_dry_run(&DryRun {
                    hook,
                    script,
                    prompt,
                    title,
                    body,
                    data: overrides,
                })
                .await?;
            if result.steps.is_empty() {
                println!("The {hook} chain has no scripts that are on.");
            }
            for step in result.steps {
                let verdict = match step.decision {
                    DryRunDecision::Pass => "nothing",
                    DryRunDecision::Reject => "reject",
                    DryRunDecision::Flag => "flag",
                    DryRunDecision::Dismiss => "dismiss",
                    DryRunDecision::Error => "FAILED",
                };
                let decides = if step.decides { "  <- decides" } else { "" };
                let log_only = if step.log_only { " (log only)" } else { "" };
                println!(
                    "{}{log_only}: {verdict} in {} µs{decides}",
                    step.name, step.micros
                );
                if let Some(detail) = step.detail {
                    println!("    {detail}");
                }
                for line in step.printed {
                    println!("    print: {line}");
                }
            }
        }
        AutomodCommand::Data(cmd) => data(&client, cmd).await?,
    }
    Ok(())
}

async fn data(client: &quill_core::Client, cmd: DataCommand) -> Result<()> {
    match cmd {
        DataCommand::List => {
            for d in client.automod_data_list().await? {
                let used = if d.used_by.is_empty() {
                    "unused".to_owned()
                } else {
                    format!("read by {}", d.used_by.join(", "))
                };
                println!(
                    "{:<24} v{:<4} {:>9} bytes  {used}  {}",
                    d.name, d.version, d.size, d.description
                );
            }
        }
        DataCommand::Get { name } => print!("{}", client.automod_data(&name).await?.content),
        DataCommand::Set {
            name,
            file,
            description,
            base,
        } => {
            let content = read_file(&file)?;
            let description = match description {
                Some(d) => d,
                None => match client.automod_data(&name).await {
                    Ok(current) => current.description,
                    Err(_) => String::new(),
                },
            };
            let saved = client
                .automod_save_data(
                    &name,
                    &SaveModerationData {
                        description,
                        content,
                        base_version: base,
                    },
                )
                .await?;
            println!("Saved {} as version {}.", saved.name, saved.version);
        }
        DataCommand::Remove { name } => {
            client.automod_delete_data(&name).await?;
            println!("Deleted {name}.");
        }
        DataCommand::History { name } => {
            for v in client.automod_data_history(&name).await? {
                let by = v.created_by.as_deref().unwrap_or("a deleted account");
                println!(
                    "v{:<4} {}  {:>9} bytes  by {by}",
                    v.version, v.created_at, v.size
                );
            }
        }
        DataCommand::Version { name, version } => {
            print!(
                "{}",
                client.automod_data_version(&name, version).await?.content
            );
        }
        DataCommand::Restore { name, version } => {
            let saved = client.automod_restore_data(&name, version).await?;
            println!(
                "Saved version {version}'s content again as version {}.",
                saved.version
            );
        }
    }
    Ok(())
}
