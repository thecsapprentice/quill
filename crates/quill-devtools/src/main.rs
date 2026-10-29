//! `quill-seed`: fills an empty Quill database with demo data for testing.
//!
//! Data goes through the server's own service functions, so it is validated,
//! sanitized and stored exactly as if real users had created it. Afterwards
//! the dates are spread over the past few months so the board looks lived in.
//!
//! This is a development tool. It is never built into the server image and
//! it refuses to touch a database that already has accounts.

mod automod;
mod content;
mod rng;

use anyhow::{Context, Result, bail};
use clap::Parser;
use quill_proto::{
    api::{CreatePersona, Credentials},
    board::{CreatePrompt, CreateReport, ReportReason, Resolution},
};
use quill_server::{
    AppState, Config, connect_and_migrate, prompts, reports, routes, server_settings, set_admin,
};
use uuid::Uuid;

use crate::rng::Rng;

/// The password every demo account shares. Demo data only.
const DEFAULT_PASSWORD: &str = "quill demo password";
const ADMIN_LOGIN: &str = "admin";
/// How far back prompt dates are spread.
const MAX_AGE_DAYS: f64 = 120.0;

/// Fills an empty Quill database with demo accounts, personas, prompts and
/// reports. Connects using QUILL_DATABASE_URL, like the server.
#[derive(Debug, Parser)]
#[command(name = "quill-seed", version)]
struct Args {
    /// Number of accounts, including the `admin` account.
    #[arg(long, default_value_t = 6, value_parser = clap::value_parser!(u16).range(4..=21))]
    accounts: u16,
    /// Number of prompts.
    #[arg(long, default_value_t = 150, value_parser = clap::value_parser!(u16).range(1..=5000))]
    prompts: u16,
    /// Seed for the random choices; the same seed gives the same content.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Password for every demo account.
    #[arg(long, env = "QUILL_DEMO_PASSWORD", default_value = DEFAULT_PASSWORD)]
    password: String,
    /// Leave out the demo moderation scripts, chains and common data.
    #[arg(long)]
    no_automod: bool,
}

struct Account {
    id: Uuid,
    login: String,
    personas: Vec<(Uuid, String)>,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run(Args::parse()).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<()> {
    let mut config = Config::from_env()?;
    // Seeding creates accounts even on a server with registration closed.
    config.registration_open = true;
    let pool = connect_and_migrate(&config.database_url).await?;

    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(&pool)
        .await?;
    if existing > 0 {
        bail!(
            "the database already has {existing} account(s); quill-seed only fills an empty \
             database. To start over with Docker, run `docker compose down -v` (this deletes \
             all data), then `docker compose up -d` and seed again"
        );
    }
    let state = AppState::new(pool.clone(), config).await?;
    // Theme tags are new tags. With approval required they arrive as tag
    // requests, which is useful to test; with new tags off, they're skipped.
    let user_tags = server_settings::settings(&state)
        .await?
        .new_tags
        .allows_new_tags();
    let mut rng = Rng::new(args.seed);

    // Accounts and personas.
    let count = usize::from(args.accounts);
    let mut logins = vec![ADMIN_LOGIN.to_owned()];
    logins.extend(
        rng.pick_distinct(content::LOGIN_NAMES, count - 1)
            .into_iter()
            .map(str::to_owned),
    );
    let persona_counts: Vec<usize> = (0..count).map(|_| rng.between(1, 3)).collect();
    let mut handles = content::handles(&mut rng, persona_counts.iter().sum()).into_iter();
    let mut accounts = Vec::with_capacity(count);
    for (login, n) in logins.into_iter().zip(persona_counts) {
        let id = routes::create_account(
            &state,
            Credentials {
                username: login.clone(),
                password: args.password.clone(),
            }
            .into(),
        )
        .await
        .with_context(|| format!("creating account {login}"))?;
        let mut personas = Vec::new();
        for (handle, display) in handles.by_ref().take(n) {
            let body = CreatePersona {
                handle: handle.clone(),
                display_name: rng.chance(80).then_some(display),
                summary: content::summary(&mut rng),
            };
            let persona = routes::add_persona(&state, id, &body)
                .await
                .with_context(|| format!("creating persona {handle}"))?;
            personas.push((persona.id, handle));
        }
        accounts.push(Account {
            id,
            login,
            personas,
        });
    }
    set_admin(&pool, ADMIN_LOGIN, true).await?;

    // Prompts, each posted by a random persona.
    let mut prompt_ids = Vec::with_capacity(usize::from(args.prompts));
    for _ in 0..args.prompts {
        let account = rng.pick(&accounts);
        let (persona, _) = rng.pick(&account.personas);
        let maturity = content::maturity(&mut rng);
        let body = CreatePrompt {
            persona: *persona,
            title: content::title(&mut rng),
            body: content::body(&mut rng),
            maturity,
            tags: content::tags(&mut rng, maturity, user_tags),
            state: Some(content::state(&mut rng)),
        };
        let prompt = prompts::post_prompt(&state, account.id, &body)
            .await
            .with_context(|| format!("posting prompt {:?}", body.title))?;
        prompt_ids.push(prompt.id);
    }

    // Spread the dates. Personas and accounts predate every prompt; about one
    // prompt in six was edited some time after posting.
    sqlx::query("UPDATE accounts SET created_at = now() - make_interval(days => 150)")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE actors SET created_at = now() - make_interval(days => 140)")
        .execute(&pool)
        .await?;
    for id in &prompt_ids {
        let age = MAX_AGE_DAYS * 86_400.0 * (rng.below(10_000) as f64 / 10_000.0);
        let edited_after = if rng.chance(15) { age * 0.3 } else { 0.0 };
        sqlx::query(
            "UPDATE prompts SET created_at = now() - make_interval(secs => $2),
                                updated_at = now() - make_interval(secs => $2) + make_interval(secs => $3)
             WHERE id = $1",
        )
        .bind(id)
        .bind(age)
        .bind(edited_after)
        .execute(&pool)
        .await?;
    }

    // A few reports, one of them already discarded by the admin.
    let reasons = [
        (
            ReportReason::Spam,
            Some("Posted the same prompt several times."),
        ),
        (
            ReportReason::WrongMaturity,
            Some("This reads as Mature, not Teen."),
        ),
        (ReportReason::Other, None),
        (
            ReportReason::Harassment,
            Some("The summary names a real person."),
        ),
    ];
    let report_count = (prompt_ids.len() / 30).clamp(3, 10).min(prompt_ids.len());
    for (i, prompt) in rng
        .pick_distinct(&prompt_ids, report_count)
        .into_iter()
        .enumerate()
    {
        let (reason, details) = reasons[i % reasons.len()];
        // The first report is anonymous, the rest come from random accounts.
        let reporter = (i > 0).then(|| rng.pick(&accounts[1..]).id);
        reports::file_report(
            &state,
            reporter,
            &CreateReport {
                prompt,
                reason,
                details: details.map(str::to_owned),
            },
        )
        .await
        .context("filing a report")?;
    }
    if let Some(first) = reports::reports(&state, false).await?.first() {
        reports::resolve(&state, accounts[0].id, first.id, Resolution::Discard).await?;
    }

    // Automated moderation, last: its demo posts are checked like anyone's.
    let automod = if args.no_automod {
        None
    } else {
        let (writer, reporter) = (&accounts[1], &accounts[2]);
        let summary = automod::seed(
            &state,
            accounts[0].id,
            writer.id,
            writer.personas[0].0,
            reporter.id,
            user_tags,
        )
        .await?;
        Some((writer.login.clone(), summary))
    };

    println!(
        "Seeded {} accounts, {} personas, {} prompts and {report_count} reports.\n",
        accounts.len(),
        accounts.iter().map(|a| a.personas.len()).sum::<usize>(),
        prompt_ids.len(),
    );
    if let Some((writer, summary)) = &automod {
        println!(
            "Automated moderation: {} scripts in chains for every hook, reading the \
             blocked_words and allowed_sites common data. {writer} posted demo prompts: \
             one flagged for shouting, one seen by a log-only script, and these refused:",
            summary.scripts,
        );
        for message in &summary.refused {
            println!("  - {message}");
        }
        println!();
    }
    println!("Every account's password: {}\n", args.password);
    for a in &accounts {
        let handles: Vec<String> = a.personas.iter().map(|(_, h)| format!("@{h}")).collect();
        let role = if a.login == ADMIN_LOGIN {
            " (admin)"
        } else {
            ""
        };
        println!("  {:<10}{role:<9} {}", a.login, handles.join(", "));
    }
    Ok(())
}
