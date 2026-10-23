//! History for devices that joined late (2k; plan: linking a new device,
//! idle devices).
//!
//! A device added to a session reads only from the commit that added it,
//! and one that was away too long misses what the server deleted. Each
//! notes the log positions it lacks ([`crate::sessions`]) and asks the
//! login's other devices over the personal device group. A device that has
//! read past them, with no gap of its own there, answers with a
//! `HistoryBundle` for the asker alone: the session's rows and files,
//! sealed as blobs in the device group (the attachment mechanism), their
//! keys in the MLS-encrypted message. The asker merges them, never dropping
//! a version, and deletes the blobs.
//!
//! Only the login's own devices take part: the other writer's devices
//! never see a request, and the server sees blobs of padded ciphertext in
//! the login's own group.

use quill_proto::keys::{b64, unb64};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    Client, CoreError, Store, attachments,
    devices::{self, Ask, BundlePart, HistoryBundle, K_ASKS, K_INBOX, Payload},
    history::{self, Rows},
    keys,
    mls::Provider,
    sessions,
};

/// The server takes at most this many blobs per message.
const BLOBS_PER_MESSAGE: usize = 10;
/// Rows go in parts of about this many bytes of JSON.
const ROWS_PART_BYTES: usize = 4 * 1024 * 1024;
/// Requests nobody could answer are dropped after this long.
const ASK_LIFETIME_MS: u64 = 30 * 24 * 60 * 60 * 1000;

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

/// What [`run`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoryReport {
    /// Sessions this device asked its other devices about.
    pub asked: usize,
    /// Requests from its other devices it answered.
    pub answered: usize,
    /// Bundles it merged.
    pub received: usize,
}

/// Merges history that arrived, asks for what's missing, and answers the
/// login's other devices. Does nothing on a device with no device group.
pub async fn run(store: &Store, client: &Client) -> Result<HistoryReport, CoreError> {
    let mut report = HistoryReport::default();
    if devices::account_device_id(store)?.is_none() {
        return Ok(report);
    }
    report.received = receive(store, client).await?;
    report.asked = ask(store, client).await?;
    report.answered = answer(store, client).await?;
    Ok(report)
}

// ---- Asking ---------------------------------------------------------------------------------

async fn ask(store: &Store, client: &Client) -> Result<usize, CoreError> {
    let mut asked = 0;
    for w in sessions::history_wanted(store)? {
        devices::send_payload(
            store,
            client,
            &Payload::HistoryRequest {
                session: b64(&w.gid),
                from: w.from,
                to: w.to,
            },
            &[],
        )
        .await?;
        sessions::history_asked(store, &w.gid)?;
        asked += 1;
    }
    Ok(asked)
}

// ---- Answering ------------------------------------------------------------------------------

async fn answer(store: &Store, client: &Client) -> Result<usize, CoreError> {
    let now = keys::now_ms();
    let asks: Vec<Ask> = devices::load_list(&Provider::new(store), K_ASKS)?;
    let mut answered = 0;
    for a in asks {
        let Some(gid) = unb64(&a.session) else {
            forget_ask(store, &a)?;
            continue;
        };
        if now.saturating_sub(a.at_ms) > ASK_LIFETIME_MS {
            forget_ask(store, &a)?;
            continue;
        }
        let Some((from, to)) = sessions::history_offer(store, &gid, a.from, a.to)? else {
            continue;
        };
        send_bundle(store, client, a.device, &gid, from, to).await?;
        forget_ask(store, &a)?;
        answered += 1;
    }
    Ok(answered)
}

fn forget_ask(store: &Store, ask: &Ask) -> Result<(), CoreError> {
    Provider::new(store).atomically(|p| {
        let mut asks: Vec<Ask> = devices::load_list(p, K_ASKS)?;
        asks.retain(|a| a != ask);
        devices::save_list(p, K_ASKS, &asks)
    })
}

/// Splits rows into parts of about [`ROWS_PART_BYTES`] of JSON each; the
/// channels and counts go with the first.
fn split(rows: Rows) -> Result<Vec<Zeroizing<Vec<u8>>>, CoreError> {
    let encode = |r: &Rows| -> Result<Zeroizing<Vec<u8>>, CoreError> {
        Ok(Zeroizing::new(
            serde_json::to_vec(r).map_err(|_| err("encoding history"))?,
        ))
    };
    let mut parts = Vec::new();
    let mut current = Rows {
        notes: Vec::new(),
        channels: rows.channels,
        seen: rows.seen,
    };
    let mut size = encode(&current)?.len();
    for note in rows.notes {
        let n = serde_json::to_vec(&note)
            .map_err(|_| err("encoding history"))?
            .len();
        if !current.notes.is_empty() && size + n > ROWS_PART_BYTES {
            parts.push(encode(&current)?);
            current = Rows::default();
            size = encode(&current)?.len();
        }
        size += n + 1;
        current.notes.push(note);
    }
    parts.push(encode(&current)?);
    Ok(parts)
}

/// Sends the asking device a session's history from `from` to `to`.
async fn send_bundle(
    store: &Store,
    client: &Client,
    for_device: Uuid,
    gid: &[u8],
    from: u64,
    to: u64,
) -> Result<(), CoreError> {
    let group = devices::device_group(store)?.ok_or_else(|| err("no device group"))?;
    let max = client.instance().await?.limits.max_attachment_bytes;
    let rows = history::export(store, gid, from, to)?;
    let mut files: Vec<(String, Zeroizing<Vec<u8>>)> = Vec::new();
    for m in &rows.notes {
        for a in &m.attachments {
            if files.iter().any(|(url, _)| *url == a.url) {
                continue;
            }
            if let Some(c) = history::file(store, &a.url)? {
                files.push((a.url.clone(), c));
            }
        }
    }
    let mut plains: Vec<(Option<String>, Zeroizing<Vec<u8>>)> =
        split(rows)?.into_iter().map(|r| (None, r)).collect();
    plains.extend(files.into_iter().map(|(url, c)| (Some(url), c)));

    let mut parts: Vec<(BundlePart, Uuid)> = Vec::new();
    for (file, plain) in plains {
        let sealed = attachments::seal(&plain, max)?;
        let up = client
            .upload_group_blob(&group, sealed.blob.clone())
            .await?;
        let name = if file.is_some() {
            "file"
        } else {
            "history.json"
        };
        parts.push((
            BundlePart {
                file,
                blob: sealed.attachment(name, client.blob_url(up.id)?, up.token),
            },
            up.id,
        ));
    }
    let messages: Vec<&[(BundlePart, Uuid)]> = parts.chunks(BLOBS_PER_MESSAGE).collect();
    let count = messages.len();
    for (i, chunk) in messages.into_iter().enumerate() {
        let bundle = HistoryBundle {
            for_device,
            session: b64(gid),
            from,
            to,
            parts: chunk.iter().map(|(p, _)| p.clone()).collect(),
            last: i + 1 == count,
        };
        let blobs: Vec<Uuid> = chunk.iter().map(|(_, id)| *id).collect();
        devices::send_payload(store, client, &Payload::HistoryBundle(bundle), &blobs).await?;
    }
    Ok(())
}

// ---- Receiving ------------------------------------------------------------------------------

async fn receive(store: &Store, client: &Client) -> Result<usize, CoreError> {
    let inbox: Vec<HistoryBundle> = devices::load_list(&Provider::new(store), K_INBOX)?;
    if inbox.is_empty() {
        return Ok(0);
    }
    let max = client.instance().await?.limits.max_attachment_bytes;
    let mut received = 0;
    for bundle in inbox {
        let gid = unb64(&bundle.session).unwrap_or_default();
        let mut rows: Vec<Rows> = Vec::new();
        let mut files: Vec<(String, Zeroizing<Vec<u8>>)> = Vec::new();
        for part in &bundle.parts {
            let blob = match client
                .fetch_blob(&part.blob.url, &part.blob.blob_token, max)
                .await
            {
                Ok(b) => b,
                // Gone (the sweep, or a duplicate already handled): what it
                // held stays missing and is asked for again later.
                Err(e) if e.status() == Some(404) => continue,
                Err(e) => return Err(e),
            };
            let Ok(plain) = attachments::open(&blob, &part.blob, max) else {
                continue;
            };
            match &part.file {
                Some(url) => files.push((url.clone(), plain)),
                None => {
                    if let Ok(r) = serde_json::from_slice::<Rows>(&plain) {
                        rows.push(r);
                    }
                }
            }
        }
        let complete = rows.len() + files.len() == bundle.parts.len();
        Provider::new(store).atomically(|p| {
            if !gid.is_empty() {
                for (url, contents) in &files {
                    history::keep_file_in(p, &gid, url, Some(contents))?;
                }
                for r in &rows {
                    history::merge(p, &gid, r)?;
                }
                if bundle.last && complete {
                    sessions::history_received(p, &gid, bundle.from, bundle.to)?;
                }
            }
            let mut inbox: Vec<HistoryBundle> = devices::load_list(p, K_INBOX)?;
            if let Some(i) = inbox
                .iter()
                .position(|b| b.session == bundle.session && b.parts == bundle.parts)
            {
                inbox.remove(i);
            }
            devices::save_list(p, K_INBOX, &inbox)
        })?;
        // Only this device needed them.
        for part in &bundle.parts {
            let _ = client
                .delete_blob(&part.blob.url, &part.blob.blob_token)
                .await;
        }
        received += 1;
    }
    Ok(received)
}
