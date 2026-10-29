// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The behaviour every [`SyncStore`] adapter must have. Each adapter's
//! tests call [`check_store`] with a factory for empty stores.

use serde_json::json;
use vellum_kernel::{ObjectId, SpaceId, UserId};

use crate::domain::{Change, ClientId};
use crate::ports::SyncStore;

/// Runs every check, returning the first failure.
///
/// # Errors
///
/// A description of the first behaviour that does not hold.
pub async fn check_store<S: SyncStore, F: Fn() -> S>(make: F) -> Result<(), String> {
    clients_bind_and_advance(&make()).await?;
    log_keeps_latest_version(&make()).await?;
    reads_page_and_skip_tombstones(&make()).await?;
    records_round_trip(&make()).await?;
    Ok(())
}

fn ensure(condition: bool, what: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(what.to_owned())
    }
}

fn client(id: &str) -> Result<ClientId, String> {
    ClientId::parse(id).map_err(|err| err.to_string())
}

async fn clients_bind_and_advance<S: SyncStore>(store: &S) -> Result<(), String> {
    let id = client("device-1")?;
    let (ada, bob) = (UserId::generate(), UserId::generate());
    ensure(
        store
            .client(&id)
            .await
            .map_err(|e| e.to_string())?
            .is_none(),
        "a new client has no state",
    )?;
    store
        .advance_client(&id, ada, 3)
        .await
        .map_err(|e| e.to_string())?;
    store
        .advance_client(&id, ada, 2)
        .await
        .map_err(|e| e.to_string())?;
    let state = store.client(&id).await.map_err(|e| e.to_string())?;
    ensure(
        state.map(|s| (s.user, s.last_mutation_id)) == Some((ada, 3)),
        "advance never moves backwards",
    )?;
    ensure(
        store.advance_client(&id, bob, 9).await.is_err(),
        "a client stays bound to its first user",
    )?;
    Ok(())
}

async fn log_keeps_latest_version<S: SyncStore>(store: &S) -> Result<(), String> {
    let space = SpaceId::generate();
    store
        .append(vec![
            Change::put(space, "issues", "ENG-1", json!(1)),
            Change::put(space, "issues", "ENG-2", json!(2)),
            Change::put(space, "issues", "ENG-1", json!(3)),
        ])
        .await
        .map_err(|e| e.to_string())?;
    let records = store.read(0, 10, false).await.map_err(|e| e.to_string())?;
    let seen: Vec<_> = records
        .iter()
        .map(|r| (r.change.key.as_str(), r.change.value.clone()))
        .collect();
    ensure(
        seen == vec![("ENG-2", Some(json!(2))), ("ENG-1", Some(json!(3)))],
        "each record appears once, at its latest write",
    )?;
    ensure(
        records
            .windows(2)
            .all(|w| matches!(w, [a, b] if a.seq < b.seq)),
        "positions increase",
    )?;
    Ok(())
}

async fn reads_page_and_skip_tombstones<S: SyncStore>(store: &S) -> Result<(), String> {
    let space = SpaceId::generate();
    store
        .append(vec![
            Change::put(space, "t", "a", json!(1)),
            Change::put(space, "t", "b", json!(2)),
            Change::delete(space, "t", "a"),
            Change::put(space, "t", "c", json!(3)),
        ])
        .await
        .map_err(|e| e.to_string())?;
    let all = store.read(0, 10, false).await.map_err(|e| e.to_string())?;
    ensure(all.len() == 3, "the delete replaced the put")?;
    let live = store.read(0, 10, true).await.map_err(|e| e.to_string())?;
    let live_keys: Vec<_> = live.iter().map(|r| r.change.key.as_str()).collect();
    ensure(live_keys == ["b", "c"], "live reads skip tombstones")?;
    let first = all.first().map_or(0, |r| r.seq);
    let page = store
        .read(first, 1, false)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        page.len() == 1 && page.first().map(|r| r.seq) > Some(first),
        "reads start after the position",
    )?;
    let info = store.info().await.map_err(|e| e.to_string())?;
    ensure(info.epoch > 0, "the epoch starts above zero")?;
    Ok(())
}

async fn records_round_trip<S: SyncStore>(store: &S) -> Result<(), String> {
    let (space, object) = (SpaceId::generate(), ObjectId::generate());
    let value = json!({"title": "Ship sync", "tags": ["a", "b"], "n": 1.5, "z": null});
    let change = Change::put(space, "issues", "ENG-7", value).governed_by(object);
    store
        .append(vec![
            change.clone(),
            Change::delete(space, "issues", "ENG-8"),
        ])
        .await
        .map_err(|e| e.to_string())?;
    let records = store.read(0, 10, false).await.map_err(|e| e.to_string())?;
    let changes: Vec<_> = records.into_iter().map(|r| r.change).collect();
    ensure(
        changes == vec![change, Change::delete(space, "issues", "ENG-8")],
        "space, object, collection, key and value come back unchanged",
    )?;
    store.append(Vec::new()).await.map_err(|e| e.to_string())?;
    Ok(())
}
