//! `quill backup`: the encrypted backup and the recovery code.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use quill_core::backup;
use serde_json::json;
use zeroize::Zeroizing;

use crate::{Ctx, keys_cmd::ask};

/// Environment variable read for the recovery code in non-interactive use
/// (tests, scripts).
const CODE_ENV: &str = "QUILL_RECOVERY_CODE";

#[derive(Subcommand)]
pub enum BackupCommand {
    /// Back up this device's personas, trust records and history to the
    /// server, sealed with a recovery code only you hold.
    Create {
        /// Make a new recovery code (the old one stops opening new backups).
        #[arg(long)]
        new_code: bool,
    },
    /// Show the login's backup on the server.
    Status,
    /// On a new device, signed in to your login: restore from the 12 words.
    /// This replaces your other devices: they're revoked and removed from
    /// your sessions.
    Restore {
        /// Don't ask before replacing the other devices.
        #[arg(long)]
        yes: bool,
    },
    /// Delete the backup from the server.
    Delete,
}

pub async fn backup(ctx: &Ctx, cmd: BackupCommand) -> Result<()> {
    let client = ctx.client()?;
    match cmd {
        BackupCommand::Create { new_code } => {
            let created = backup::create(&ctx.store, &client, new_code).await?;
            let words = created.code.as_ref().map(|w| w.join(" "));
            let body = json!({
                "generation": created.info.generation,
                "bytes": created.info.bytes,
                "recoveryCode": words,
            });
            ctx.print(&body, |_| {
                println!("backed up ({})", crate::size(created.info.bytes));
                if let Some(words) = &words {
                    println!();
                    println!("Your recovery code. Write it down and keep it somewhere safe:");
                    println!();
                    let w: Vec<&str> = words.split(' ').collect();
                    println!("    {}", w[..6].join(" "));
                    println!("    {}", w[6..].join(" "));
                    println!();
                    println!("It's shown only now. Without it, nobody (not even the server's");
                    println!("administrators) can open the backup. This device keeps it to");
                    println!("refresh the backup with `quill backup create`.");
                }
            })
        }
        BackupCommand::Status => {
            let info = client.backup_info().await?;
            let keeps = backup::has_code(&ctx.store)?;
            ctx.print(&json!({ "backup": info, "codeOnThisDevice": keeps }), |_| {
                match &info {
                    Some(b) => println!(
                        "backup of {} made {}",
                        crate::size(b.bytes),
                        b.created_at.date()
                    ),
                    None => println!("no backup on the server"),
                }
                if !keeps {
                    println!("this device doesn't hold a recovery code: `quill backup create` makes one");
                }
            })
        }
        BackupCommand::Restore { yes } => {
            if !yes
                && !ask(
                    "Restoring replaces your login's other devices: they're revoked and removed from your sessions. Go on?",
                )?
            {
                bail!("not restored");
            }
            let code = match std::env::var(CODE_ENV) {
                Ok(c) => Zeroizing::new(c),
                Err(_) => Zeroizing::new(
                    rpassword::prompt_password("Recovery code (12 words): ")
                        .with_context(|| format!("couldn't ask for the code; set {CODE_ENV}"))?,
                ),
            };
            let restored = backup::restore(&ctx.store, &client, &code).await?;
            let body = json!({
                "personas": restored.personas,
                "revoked": restored.revoked,
                "rejoins": restored.rejoins,
            });
            ctx.print(&body, |_| {
                println!("restored: {}", restored.personas.iter().map(|p| format!("@{p}")).collect::<Vec<_>>().join(", "));
                println!("{} lost devices revoked", restored.revoked);
                println!(
                    "asked to rejoin {} sessions: the other writers' clients let this device in when they next run",
                    restored.rejoins
                );
            })
        }
        BackupCommand::Delete => {
            client.delete_backup().await?;
            ctx.done("backup deleted")
        }
    }
}
