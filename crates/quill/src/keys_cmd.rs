//! `quill keys` and `quill device`: persona identity keys and devices.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use quill_certs::RevocationReason;
use quill_core::keys::{self, CheckedKeys, DeviceStatus, PinStatus};
use quill_proto::{api::Persona, keys::actor_uri};
use serde_json::json;
use uuid::Uuid;

use crate::Ctx;

#[derive(Subcommand)]
pub enum KeysCommand {
    /// Create and publish a persona's identity key and this device's key.
    Init {
        /// The persona (handle).
        #[arg(long = "as", value_name = "HANDLE")]
        persona: String,
        /// Replace an identity key made on another device. Its other devices
        /// stop working, and other writers are told the key changed.
        #[arg(long)]
        new_identity: bool,
    },
    /// Top up this device's pool of KeyPackages on the server, so others
    /// can add it to sessions while it's offline.
    Publish {
        #[arg(long = "as", value_name = "HANDLE")]
        persona: String,
        /// One-time packages to keep waiting.
        #[arg(long, default_value_t = keys::POOL_TARGET)]
        count: u32,
    },
    /// How many KeyPackages this device has waiting.
    Status {
        #[arg(long = "as", value_name = "HANDLE")]
        persona: String,
    },
    /// For testing: claim and check one KeyPackage per device of a persona.
    /// Uses up one of each device's packages.
    Claim { handle: String },
    /// Fetch and check any persona's published keys.
    Show {
        handle: String,
        /// Trust a changed identity key (after checking its fingerprint with
        /// its owner some other way).
        #[arg(long)]
        accept_change: bool,
    },
}

#[derive(Subcommand)]
pub enum DeviceCommand {
    /// List a persona's devices and check their certificates.
    List {
        #[arg(long = "as", value_name = "HANDLE")]
        persona: String,
    },
    /// Revoke one of a persona's devices. Permanent.
    Revoke {
        device: Uuid,
        #[arg(long = "as", value_name = "HANDLE")]
        persona: String,
        /// lost, compromised, retired or unspecified.
        #[arg(long, default_value = "unspecified")]
        reason: RevocationReason,
    },
}

/// A date for display, from seconds since the Unix epoch.
fn date(secs: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp(i64::try_from(secs).unwrap_or(0))
        .map(|t| t.date().to_string())
        .unwrap_or_else(|_| secs.to_string())
}

fn bare(handle: &str) -> &str {
    handle.trim_start_matches('@')
}

async fn own_persona(ctx: &Ctx, handle: &str) -> Result<Persona> {
    let handle = bare(handle);
    ctx.client()?
        .personas()
        .await?
        .into_iter()
        .find(|p| p.handle == handle)
        .with_context(|| format!("you have no persona @{handle}"))
}

fn status_text(s: &DeviceStatus) -> String {
    match s {
        DeviceStatus::Valid { limited: false } => "valid".into(),
        DeviceStatus::Valid { limited: true } => "valid, limited".into(),
        DeviceStatus::Superseded => "needs certifying again (older identity key)".into(),
        DeviceStatus::Revoked(r) => format!("revoked ({})", r.as_str()),
        DeviceStatus::Invalid(why) => format!("INVALID: {why}"),
    }
}

fn status_word(s: &DeviceStatus) -> &'static str {
    match s {
        DeviceStatus::Valid { .. } => "valid",
        DeviceStatus::Superseded => "superseded",
        DeviceStatus::Revoked(_) => "revoked",
        DeviceStatus::Invalid(_) => "invalid",
    }
}

fn devices_json(checked: &CheckedKeys, mine: Option<[u8; 32]>) -> serde_json::Value {
    checked
        .devices
        .iter()
        .map(|d| {
            json!({
                "id": d.id,
                "status": status_word(&d.status),
                "detail": status_text(&d.status),
                "thisDevice": mine.is_some_and(|m| m.as_slice() == d.device_key),
            })
        })
        .collect()
}

fn print_devices(checked: &CheckedKeys, mine: Option<[u8; 32]>) {
    if checked.devices.is_empty() {
        println!("no devices");
    }
    for d in &checked.devices {
        let here = if mine.is_some_and(|m| m.as_slice() == d.device_key) {
            "  (this device)"
        } else {
            ""
        };
        println!("  {}  {}{here}", d.id, status_text(&d.status));
    }
}

pub async fn keys(ctx: &Ctx, cmd: KeysCommand) -> Result<()> {
    match cmd {
        KeysCommand::Init {
            persona,
            new_identity,
        } => {
            let p = own_persona(ctx, &persona).await?;
            let client = ctx.client()?;
            let out = keys::init(&ctx.store, &client, &p, new_identity).await?;
            let pool = keys::publish(&ctx.store, &client, &p, keys::POOL_TARGET).await?;
            let body = json!({
                "persona": p.handle,
                "fingerprint": out.fingerprint,
                "identityKeyVersion": out.identity_version,
                "newIdentity": out.new_identity,
                "device": out.device,
                "newDevice": out.new_device,
                "keyPackages": pool.available,
            });
            ctx.print(&body, |_| {
                println!("@{}", p.handle);
                println!(
                    "identity key: {} (version {}){}",
                    out.fingerprint,
                    out.identity_version,
                    if out.new_identity { ", published now" } else { "" }
                );
                println!(
                    "this device: {}{}",
                    out.device,
                    if out.new_device { ", certified now" } else { "" }
                );
                println!("KeyPackages waiting: {}", pool.available);
                if out.new_identity && out.identity_version > 1 {
                    println!(
                        "The identity key changed: this persona's other devices need linking again, and other writers will see a warning."
                    );
                }
            })
        }
        KeysCommand::Publish { persona, count } => {
            let p = own_persona(ctx, &persona).await?;
            let out = keys::publish(&ctx.store, &ctx.client()?, &p, count).await?;
            let body = json!({
                "uploaded": out.uploaded,
                "lastResortRenewed": out.last_resort_renewed,
                "available": out.available,
                "lastResortUntil": out.last_resort_until,
            });
            ctx.print(&body, |_| {
                println!(
                    "published {} KeyPackage(s){}; {} waiting",
                    out.uploaded,
                    if out.last_resort_renewed {
                        " and a new last-resort one"
                    } else {
                        ""
                    },
                    out.available
                );
            })
        }
        KeysCommand::Status { persona } => {
            let p = own_persona(ctx, &persona).await?;
            let device = keys::load_device(&ctx.store, p.id)?
                .and_then(|d| d.id)
                .with_context(|| format!("this device has no keys for @{}", p.handle))?;
            let status = ctx.client()?.key_package_status(p.id, device).await?;
            ctx.print(&status, |s| {
                println!("KeyPackages waiting: {}", s.available);
                match s.last_resort_until {
                    Some(t) => println!("last-resort package until {}", date(t)),
                    None => println!(
                        "no last-resort package: run `quill keys publish --as {}`",
                        p.handle
                    ),
                }
            })
        }
        KeysCommand::Claim { handle } => {
            let handle = bare(&handle).to_owned();
            let client = ctx.client()?;
            let domain = client.instance().await?.domain;
            let uri = actor_uri(&domain, &handle);
            let claim = keys::claim(&ctx.store, &client, &handle, &uri).await?;
            let body = json!({
                "accepted": claim.packages.iter().map(|p| json!({
                    "device": p.device, "lastResort": p.last_resort,
                })).collect::<Vec<_>>(),
                "refused": claim.refused.iter().map(|(d, why)| json!({
                    "device": d, "reason": why,
                })).collect::<Vec<_>>(),
                "missing": claim.missing,
            });
            ctx.print(&body, |_| {
                for p in &claim.packages {
                    let lr = if p.last_resort { " (last resort)" } else { "" };
                    println!("  {}  KeyPackage checks out{lr}", p.device);
                }
                for (d, why) in &claim.refused {
                    println!("  {d}  REFUSED: {why}");
                }
                for d in &claim.missing {
                    println!("  {d}  no KeyPackage available");
                }
            })?;
            if !claim.refused.is_empty() {
                bail!("some KeyPackages failed their checks");
            }
            Ok(())
        }
        KeysCommand::Show {
            handle,
            accept_change,
        } => {
            let handle = bare(&handle).to_owned();
            let client = ctx.client()?;
            let domain = client.instance().await?.domain;
            let uri = actor_uri(&domain, &handle);
            let published = client.actor_keys(&handle).await?;
            let checked = keys::check_published(&published, &uri, keys::now_ms())?;
            let pin = match &checked.identity {
                Some((key, version)) => {
                    let status = keys::check_pin(&ctx.store, &uri, key, *version)?;
                    if accept_change && matches!(status, PinStatus::Changed { .. }) {
                        keys::accept_pin(&ctx.store, &uri, key, *version)?;
                    }
                    Some(status)
                }
                None => None,
            };
            let changed = !accept_change && matches!(pin, Some(PinStatus::Changed { .. }));
            let body = json!({
                "handle": handle,
                "actorUri": uri,
                "fingerprint": checked.fingerprint(),
                "identityKeyVersion": checked.identity.as_ref().map(|(_, v)| v),
                "pin": match &pin {
                    None => "none",
                    Some(PinStatus::New) => "new",
                    Some(PinStatus::Matches) => "matches",
                    Some(PinStatus::Changed { .. }) if accept_change => "accepted",
                    Some(PinStatus::Changed { .. }) => "changed",
                },
                "devices": devices_json(&checked, None),
            });
            ctx.print(&body, |_| {
                println!("@{handle}  {uri}");
                match (checked.fingerprint(), &checked.identity) {
                    (Some(fp), Some((_, v))) => println!("identity key: {fp} (version {v})"),
                    _ => println!("no identity key yet: this persona can't take part in sessions"),
                }
                match &pin {
                    Some(PinStatus::New) => println!("first time seen: pinned"),
                    Some(PinStatus::Matches) => println!("matches the key pinned earlier"),
                    Some(PinStatus::Changed {
                        pinned_fingerprint,
                        pinned_version,
                    }) => {
                        println!(
                            "WARNING: the identity key CHANGED. Pinned earlier: {pinned_fingerprint} (version {pinned_version})."
                        );
                        if accept_change {
                            println!("Accepted: the new key is pinned now.");
                        } else {
                            println!(
                                "Check the new fingerprint with its owner some other way, then run again with --accept-change."
                            );
                        }
                    }
                    None => {}
                }
                print_devices(&checked, None);
            })?;
            if changed {
                bail!("@{handle}'s identity key changed since it was pinned");
            }
            Ok(())
        }
    }
}

pub async fn device(ctx: &Ctx, cmd: DeviceCommand) -> Result<()> {
    match cmd {
        DeviceCommand::List { persona } => {
            let p = own_persona(ctx, &persona).await?;
            let published = ctx.client()?.persona_keys(p.id).await?;
            let checked = keys::check_published(&published, &p.actor_uri, keys::now_ms())?;
            let mine = keys::load_device(&ctx.store, p.id)?.map(|d| d.public());
            let body = json!({
                "persona": p.handle,
                "fingerprint": checked.fingerprint(),
                "devices": devices_json(&checked, mine),
            });
            ctx.print(&body, |_| {
                println!("@{}", p.handle);
                match checked.fingerprint() {
                    Some(fp) => println!("identity key: {fp}"),
                    None => println!(
                        "no identity key yet; run `quill keys init --as {}`",
                        p.handle
                    ),
                }
                print_devices(&checked, mine);
            })
        }
        DeviceCommand::Revoke {
            device,
            persona,
            reason,
        } => {
            let p = own_persona(ctx, &persona).await?;
            let info = keys::revoke(&ctx.store, &ctx.client()?, &p, device, reason).await?;
            ctx.print(&info, |i| {
                println!("revoked {} ({})", i.id, reason.as_str())
            })
        }
    }
}
