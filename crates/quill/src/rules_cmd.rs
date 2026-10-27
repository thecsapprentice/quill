//! CLI commands for the server rules: reading and accepting them (anyone),
//! and writing them (`quill admin rules-set`).

use anyhow::{Result, bail};
use clap::Subcommand;
use quill_proto::rules::{Rules, UpdateRules};

use crate::{Ctx, board::BodyArgs};

#[derive(Subcommand)]
pub enum RulesCommand {
    /// Show the server rules, and whether you must accept them.
    Show,
    /// Accept the server rules, naming the version you read (`quill rules show`).
    Accept { version: u32 },
}

fn print_rules(r: &Rules) {
    println!("version {}, updated {}", r.version, r.updated_at.date());
    println!();
    println!("{}", r.body);
    if r.must_accept {
        println!();
        println!(
            "These rules changed since you joined. To keep posting, run: quill rules accept {}",
            r.version
        );
    }
}

pub async fn rules(ctx: &Ctx, cmd: RulesCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        RulesCommand::Show => {
            let r = client.rules().await?;
            ctx.print(&r, print_rules)
        }
        RulesCommand::Accept { version } => {
            client.accept_rules(version).await?;
            if !ctx.json {
                println!("accepted the server rules (version {version})");
            }
            Ok(())
        }
    }
}

/// `quill admin rules-set`: saves the next version of the rules.
pub async fn set(ctx: &Ctx, body: &BodyArgs, minor: bool) -> Result<()> {
    let Some(text) = body.read()? else {
        bail!("give the rules with --body or --body-file");
    };
    let r = ctx
        .client()?
        .admin_update_rules(&UpdateRules { body: text, minor })
        .await?;
    ctx.print(&r, |r| {
        println!(
            "saved the server rules as version {}{}",
            r.version,
            if minor { " (minor edit)" } else { "" }
        );
    })
}
