//! Live delivery (2g): `GET /events`, a server-sent event stream of
//! content-free notices for the caller's login.
//!
//! Notices go through one in-process broadcast channel, each addressed to
//! the logins it concerns; every stream passes on only its own login's.
//! (A server running several processes would carry them over Postgres
//! `LISTEN`/`NOTIFY` instead.)

use std::{convert::Infallible, time::Duration};

use axum::{
    Router,
    extract::State,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
};
use futures_util::stream::{self, Stream};
use quill_proto::events::Event;
use sqlx::PgConnection;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{AppState, auth::Authed, error::ApiError};

/// Notices held for slow streams before they're told to resync.
const BUFFER: usize = 1024;

/// A notice and the logins it's for.
#[derive(Debug, Clone)]
pub struct Notice {
    pub accounts: Vec<Uuid>,
    pub event: Event,
}

/// The broadcast channel's sending side, kept in [`AppState`].
#[derive(Debug, Clone)]
pub struct Events(broadcast::Sender<Notice>);

impl Default for Events {
    fn default() -> Self {
        Self(broadcast::channel(BUFFER).0)
    }
}

impl Events {
    /// Sends `event` to `accounts`' open streams. Nobody listening is fine.
    pub fn send(&self, accounts: Vec<Uuid>, event: Event) {
        if !accounts.is_empty() {
            let _ = self.0.send(Notice { accounts, event });
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<Notice> {
        self.0.subscribe()
    }
}

/// The logins that can read a group: a personal group's, or its session
/// members'.
pub(crate) async fn group_accounts(
    conn: &mut PgConnection,
    group: &[u8],
) -> Result<Vec<Uuid>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT account_id FROM mls_groups WHERE group_id = $1 AND account_id IS NOT NULL
         UNION
         SELECT a.account_id FROM session_members m JOIN actors a ON a.id = m.actor_id
         WHERE m.group_id = $1",
    )
    .bind(group)
    .fetch_all(conn)
    .await?)
}

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(events))
}

fn sse(event: &Event) -> SseEvent {
    SseEvent::default()
        .event(event.name())
        .data(serde_json::to_string(event).unwrap_or_default())
}

/// `GET /events`: the stream. Kept alive with a comment every 15 seconds.
async fn events(
    State(state): State<AppState>,
    authed: Authed,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let account = authed.account_id;
    let rx = state.events.subscribe();
    let stream = stream::unfold(rx, move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(n) if n.accounts.contains(&account) => return Some((Ok(sse(&n.event)), rx)),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    return Some((Ok(sse(&Event::Resync)), rx));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
