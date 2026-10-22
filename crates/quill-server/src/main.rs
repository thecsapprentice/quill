use anyhow::{Context, Result, bail};
use quill_server::{AppState, Config, connect_and_migrate, router, set_admin};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage:
  quill-server                     run the server (configured by QUILL_* variables)
  quill-server make-admin <login>  give an account administrator rights
  quill-server revoke-admin <login>";

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    // Logs carry no client IPs, request bodies or headers; nothing per request
    // is logged except internal errors.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config::from_env()?;
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["serve"] => serve(config).await,
        [cmd @ ("make-admin" | "revoke-admin"), login] => {
            let pool = connect_and_migrate(&config.database_url).await?;
            let grant = *cmd == "make-admin";
            if !set_admin(&pool, login, grant).await? {
                bail!("no account with login name {login:?}");
            }
            println!(
                "{login} is {} an administrator",
                if grant { "now" } else { "no longer" }
            );
            Ok(())
        }
        _ => bail!("{USAGE}"),
    }
}

async fn serve(config: Config) -> Result<()> {
    let bind = config.bind;
    let pool = connect_and_migrate(&config.database_url).await?;
    let state = AppState::new(pool, config).await?;

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "quill-server listening");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("server error")
}
