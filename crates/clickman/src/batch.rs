use rusqlite::{Connection, Transaction};
use serde_json::Value;

use crate::clock::millis;
use crate::config::Config;
use crate::error::Result;
use crate::store::sql_count;

/// The oldest waiting rows, taken to send together: a run of track events, or
/// one visitor link alone (identity.json); `last_seq` is the newest of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Batch {
    pub(crate) last_seq: i64,
    pub(crate) bodies: Vec<String>,
    pub(crate) kind: Kind,
}

/// What a take sends: events to `/v1/batch` or a link to `/v1/identify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Events,
    Link,
}

/// Whether a batch goes only when due, or whenever anything waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Urgency {
    Due,
    Forced,
}

impl Batch {
    /// `{"sentAt": …, "batch": [bodies]}`, the bodies verbatim.
    pub(crate) fn payload(&self, sent_at: &str) -> String {
        format!(
            r#"{{"sentAt":"{sent_at}","batch":[{}]}}"#,
            self.bodies.join(",")
        )
    }

    /// The request body of a link: the stored row without its `type`.
    pub(crate) fn link_body(&self) -> String {
        let link: Value = serde_json::from_str(&self.bodies[0]).unwrap_or_default();
        serde_json::json!({
            "anonymousId": link["anonymousId"],
            "externalId": link["externalId"],
        })
        .to_string()
    }

    fn cut(mut rows: Vec<(i64, String)>, config: &Config) -> Option<Self> {
        let first_is_link = is_link(&rows.first()?.1);
        let (kind, count) = if first_is_link {
            (Kind::Link, 1)
        } else {
            let run = rows.iter().take_while(|(_, body)| !is_link(body)).count();
            let bodies = rows[..run].iter().map(|(_, body)| body.as_str());
            let count = taken(bodies, config.max_batch_events, config.max_batch_bytes);
            (Kind::Events, count)
        };
        rows.truncate(count);
        let last_seq = rows.last()?.0;
        let bodies = rows.into_iter().map(|(_, body)| body).collect();
        Some(Self {
            last_seq,
            bodies,
            kind,
        })
    }
}

/// Whether a stored row is a visitor link rather than a track event.
pub(crate) fn is_link(body: &str) -> bool {
    serde_json::from_str::<Value>(body).is_ok_and(|row| row["type"] == "identify")
}

/// The next batch, or `None` when nothing is due; events older than `max_age`
/// are deleted unsent first. `now` is in milliseconds since the Unix epoch.
pub(crate) fn take(
    transaction: &Transaction,
    config: &Config,
    now: i64,
    urgency: Urgency,
) -> Result<Option<Batch>> {
    purge(transaction, now.saturating_sub(millis(config.max_age)))?;
    if !is_due(transaction, config, now, urgency)? {
        return Ok(None);
    }
    let rows = oldest(transaction, config.max_batch_events)?;
    Ok(Batch::cut(rows, config))
}

/// How many of the oldest `bodies` one batch takes: at most `max_events`, while
/// they stay within `max_bytes` UTF-8 bytes joined by commas; the oldest goes
/// even alone over the limit (batches.json).
pub(crate) fn taken<'a>(
    bodies: impl IntoIterator<Item = &'a str>,
    max_events: usize,
    max_bytes: usize,
) -> usize {
    let mut joined = 0;
    bodies
        .into_iter()
        .take(max_events)
        .enumerate()
        .take_while(|(index, body)| {
            joined += body.len() + usize::from(*index > 0);
            *index == 0 || joined <= max_bytes
        })
        .count()
}

/// Deletes a delivered or refused batch; events tracked since keep their place.
pub(crate) fn remove(transaction: &Transaction, batch: &Batch) -> Result<()> {
    transaction.execute("DELETE FROM events WHERE seq <= ?1", [batch.last_seq])?;
    Ok(())
}

pub(crate) fn clear(transaction: &Transaction) -> Result<()> {
    transaction.execute("DELETE FROM events", [])?;
    Ok(())
}

pub(crate) fn pending(connection: &Connection) -> Result<usize> {
    Ok(connection.query_row("SELECT count(*) FROM events", [], |row| row.get(0))?)
}

pub(crate) fn purge(transaction: &Transaction, created_before: i64) -> Result<()> {
    let purged =
        transaction.execute("DELETE FROM events WHERE created_at < ?1", [created_before])?;
    if purged > 0 {
        log::warn!("ClickMan deleted {purged} events that waited past max_age unsent");
    }
    Ok(())
}

fn is_due(connection: &Connection, config: &Config, now: i64, urgency: Urgency) -> Result<bool> {
    let (waiting, oldest): (usize, Option<i64>) =
        connection.query_row("SELECT count(*), min(created_at) FROM events", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    Ok(match (oldest, urgency) {
        (None, _) => false,
        (Some(_), Urgency::Forced) => true,
        (Some(oldest), Urgency::Due) => {
            waiting >= config.flush_at
                || now.saturating_sub(oldest) >= millis(config.flush_interval)
        }
    })
}

fn oldest(connection: &Connection, max_events: usize) -> Result<Vec<(i64, String)>> {
    let mut statement = connection.prepare("SELECT seq, body FROM events ORDER BY seq LIMIT ?1")?;
    let rows = statement.query_map([sql_count(max_events)], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
