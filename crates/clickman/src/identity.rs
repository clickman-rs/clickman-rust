use rusqlite::{Connection, Transaction};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::event::{Recorder, is_valid_text};

pub(crate) const MAX_EXTERNAL_ID_CHARS: usize = 256;

/// Whom later events are about: their `externalId` and the traits their context carries.
pub(crate) struct Actor {
    pub(crate) external_id: String,
    pub(crate) traits: Map<String, Value>,
}

pub(crate) fn actor(connection: &Connection) -> Result<Actor> {
    let (external_id, traits): (String, String) = connection.query_row(
        "SELECT external_id, traits FROM identity WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let traits = serde_json::from_str(&traits).map_err(Error::StoredTraits)?;
    Ok(Actor {
        external_id,
        traits,
    })
}

/// The prefix of a visitor's `externalId`, as the browser SDK makes them.
const VISITOR: &str = "anon:";
const ANONYMOUS: &str = "*";

/// Makes `external_id` the actor of later events. Identifying an account while
/// the store tracks as a visitor queues the link between them (identity.json).
pub(crate) fn identify(recorder: &Recorder, external_id: &str) -> Result<()> {
    if !is_valid_text(external_id, MAX_EXTERNAL_ID_CHARS) || external_id.starts_with(VISITOR) {
        return Err(Error::InvalidExternalId);
    }
    let current = actor(recorder.transaction)?.external_id;
    if current == external_id {
        return Ok(());
    }
    if current.starts_with(VISITOR) && external_id != ANONYMOUS {
        recorder.link(&current, external_id)?;
    }
    recorder.transaction.execute(
        "UPDATE identity SET external_id = ?1 WHERE id = 1",
        [external_id],
    )?;
    Ok(())
}

pub(crate) fn identify_installation(transaction: &Transaction) -> Result<()> {
    transaction.execute(
        "UPDATE identity SET external_id = ?1 WHERE id = 1 AND external_id = '*'",
        [format!("{VISITOR}{}", Uuid::new_v4())],
    )?;
    Ok(())
}

/// Makes later events anonymous and forgets the traits.
pub(crate) fn reset(transaction: &Transaction) -> Result<()> {
    transaction.execute(
        "UPDATE identity SET external_id = '*', traits = '{}' WHERE id = 1",
        [],
    )?;
    Ok(())
}

pub(crate) fn set_traits(transaction: &Transaction, changes: Map<String, Value>) -> Result<()> {
    let traits = merge(actor(transaction)?.traits, changes);
    transaction.execute(
        "UPDATE identity SET traits = ?1 WHERE id = 1",
        [Value::Object(traits).to_string()],
    )?;
    Ok(())
}

/// Traits merged key by key: a null removes a trait, any other value replaces it whole (traits.json).
pub(crate) fn merge(
    mut traits: Map<String, Value>,
    changes: Map<String, Value>,
) -> Map<String, Value> {
    traits.extend(changes);
    traits.retain(|_, value| !value.is_null());
    traits
}
