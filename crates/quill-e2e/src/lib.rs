//! Test harness: starts a real `quill-server` on a random port, backed by a
//! fresh Postgres database that is dropped afterwards.
//!
//! Set `QUILL_TEST_DATABASE_URL` to a Postgres URL whose role may create
//! databases, e.g. `postgres://quill:quill@localhost/postgres`. If it is
//! unset, tests are skipped locally but fail in CI (`CI` is set).

use std::net::SocketAddr;

use anyhow::{Context, Result};
use quill_core::Client;
use quill_proto::api::Credentials;
use quill_server::{AppState, Config, connect_and_migrate, router};
use sqlx::{AssertSqlSafe, Connection, PgConnection};
use tokio::task::JoinHandle;

pub const DATABASE_ENV: &str = "QUILL_TEST_DATABASE_URL";

/// One request and its response, as the server answered it.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub method: String,
    /// Without the query, as requested (`/api/v1/...`).
    pub path: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// What a recording server answered, in order.
#[derive(Debug, Clone, Default)]
pub struct Recording(std::sync::Arc<std::sync::Mutex<Vec<Exchange>>>);

impl Recording {
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.0.lock().map(|e| e.clone()).unwrap_or_default()
    }
}

async fn record(
    recording: Recording,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_owned();
    let response = next.run(req).await;
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if content_type.as_deref() == Some("text/event-stream") {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    if let Ok(mut e) = recording.0.lock() {
        e.push(Exchange {
            method,
            path,
            status: parts.status.as_u16(),
            content_type,
            body: bytes.to_vec(),
        });
    }
    axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
}

/// A running server plus the database it owns.
pub struct TestServer {
    pub addr: SocketAddr,
    client: Client,
    db_url: String,
    admin_url: String,
    db_name: String,
    task: JoinHandle<()>,
}

impl TestServer {
    /// Starts a server, or returns `None` when no test database is configured
    /// outside CI.
    pub async fn start() -> Result<Option<Self>> {
        Self::start_with(|_| {}).await
    }

    /// Like [`TestServer::start`], letting the caller adjust the config.
    pub async fn start_with(tweak: impl FnOnce(&mut Config)) -> Result<Option<Self>> {
        Self::start_inner(tweak, None).await
    }

    /// Like [`TestServer::start`], keeping every API exchange (the
    /// streams of `GET /events` aside) so a test can check them against
    /// the OpenAPI file.
    pub async fn start_recording() -> Result<Option<(Self, Recording)>> {
        let recording = Recording::default();
        Ok(Self::start_inner(|_| {}, Some(recording.clone()))
            .await?
            .map(|s| (s, recording)))
    }

    async fn start_inner(
        tweak: impl FnOnce(&mut Config),
        recording: Option<Recording>,
    ) -> Result<Option<Self>> {
        let Ok(admin_url) = std::env::var(DATABASE_ENV) else {
            if std::env::var_os("CI").is_some() {
                anyhow::bail!("{DATABASE_ENV} must be set in CI");
            }
            eprintln!("skipping: {DATABASE_ENV} not set");
            return Ok(None);
        };

        let db_name = format!("quill_test_{}", uuid::Uuid::now_v7().simple());
        let mut admin = PgConnection::connect(&admin_url)
            .await
            .context("connecting to test Postgres")?;
        // Safe: `db_name` is a fixed prefix plus a hex UUID we generated.
        sqlx::raw_sql(AssertSqlSafe(format!("CREATE DATABASE {db_name}")))
            .execute(&mut admin)
            .await?;
        admin.close().await?;

        let db_url = with_database(&admin_url, &db_name)?;
        let pool = connect_and_migrate(&db_url).await?;
        let mut config = Config::for_tests(db_url.clone());
        tweak(&mut config);
        let state = AppState::new(pool, config).await?;

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let mut app = router(state);
        if let Some(recording) = recording {
            app = app.layer(axum::middleware::from_fn(move |req, next| {
                record(recording.clone(), req, next)
            }));
        }
        let task = tokio::spawn(async move {
            let _ = axum_serve(listener, app).await;
        });
        let client = Client::new(&format!("http://{addr}"))?;
        Ok(Some(Self {
            addr,
            client,
            db_url,
            admin_url,
            db_name,
            task,
        }))
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// An unauthenticated client for this server.
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// Registers `username` with a fixed test password and returns a client
    /// holding its token.
    pub async fn account(&self, username: &str) -> Result<Client> {
        let token = self
            .client()
            .register(&Credentials {
                username: username.into(),
                password: TEST_PASSWORD.into(),
            })
            .await?;
        Ok(self.client().with_token(token.token))
    }

    /// Registers `username` as a server administrator and returns its client.
    pub async fn admin_account(&self, username: &str) -> Result<Client> {
        let client = self.account(username).await?;
        let pool = sqlx::PgPool::connect(&self.db_url).await?;
        anyhow::ensure!(quill_server::set_admin(&pool, username, true).await?);
        pool.close().await;
        Ok(client)
    }

    /// Server state on this server's database, for calling server functions
    /// directly, as the `quill-server` admin commands do.
    pub async fn state(&self) -> Result<AppState> {
        self.state_with(|_| {}).await
    }

    /// Like [`TestServer::state`], with the config adjusted, as a server
    /// restarted with different settings would see it.
    pub async fn state_with(&self, tweak: impl FnOnce(&mut Config)) -> Result<AppState> {
        let pool = sqlx::PgPool::connect(&self.db_url).await?;
        let mut config = Config::for_tests(self.db_url.clone());
        tweak(&mut config);
        AppState::new(pool, config).await
    }

    /// Runs SQL against this server's database (for inspecting state).
    pub async fn db(&self) -> Result<sqlx::PgPool> {
        Ok(sqlx::PgPool::connect(&self.db_url).await?)
    }

    /// Stops the server and drops its database.
    pub async fn stop(self) -> Result<()> {
        self.task.abort();
        let _ = self.task.await;
        let mut admin = PgConnection::connect(&self.admin_url).await?;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.db_name
        )))
        .execute(&mut admin)
        .await?;
        Ok(())
    }
}

pub const TEST_PASSWORD: &str = "correct horse battery staple";

async fn axum_serve(listener: tokio::net::TcpListener, app: axum::Router) -> Result<()> {
    axum::serve(listener, app).await?;
    Ok(())
}

/// Replaces the database name in a Postgres URL.
fn with_database(url: &str, db: &str) -> Result<String> {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (url, None),
    };
    let (prefix, _) = base
        .rsplit_once('/')
        .context("database URL has no database name")?;
    Ok(match query {
        Some(q) => format!("{prefix}/{db}?{q}"),
        None => format!("{prefix}/{db}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_rewrite() {
        assert_eq!(
            with_database("postgres://u:p@localhost/postgres", "x").unwrap(),
            "postgres://u:p@localhost/x"
        );
        assert_eq!(
            with_database("postgres://u@h:5432/postgres?sslmode=disable", "x").unwrap(),
            "postgres://u@h:5432/x?sslmode=disable"
        );
    }
}
