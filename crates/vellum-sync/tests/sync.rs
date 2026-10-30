// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Push, pull and publish against the in-memory store.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helpers fail loudly"
)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vellum_kernel::{Error, ErrorKind, RequestId, SpaceId, UserId};
use vellum_sync::conformance::check_store;
use vellum_sync::memory::MemoryStore;
use vellum_sync::ports::BoxFuture;
use vellum_sync::{
    AccessKey, Change, ClientId, ClientMutation, MAX_PUSH, MutationContext,
    Mutators, PULL_PAGE, Patch, PullRequest, PushRequest, ReadAccess,
    ServerMutator, Sync,
};

/// Read access: the user reads every space in `allowed`, but no object
/// with narrower permissions.
#[derive(Default)]
struct Access {
    allowed: Mutex<HashSet<SpaceId>>,
    epoch: AtomicU64,
}

/// Shared handle, so tests can change permissions under a running engine.
#[derive(Clone)]
struct Shared(Arc<Access>);

impl std::ops::Deref for Shared {
    type Target = Access;
    fn deref(&self) -> &Access {
        &self.0
    }
}

impl ReadAccess for Shared {
    async fn epoch(&self, _user: UserId) -> Result<u64, Error> {
        Ok(self.epoch.load(Ordering::SeqCst))
    }

    async fn readable(
        &self,
        _user: UserId,
        keys: &[AccessKey],
    ) -> Result<HashSet<AccessKey>, Error> {
        let allowed = self.allowed.lock().unwrap().clone();
        Ok(keys
            .iter()
            .copied()
            .filter(|key| allowed.contains(&key.space) && key.object.is_none())
            .collect())
    }
}

/// `issues.set {key, value}` writes one record; failures on request.
struct SetIssue {
    space: SpaceId,
    calls: AtomicUsize,
}

impl ServerMutator for SetIssue {
    fn apply<'a>(
        &'a self,
        context: &'a MutationContext,
        args: Value,
    ) -> BoxFuture<'a, Result<Vec<Change>, Error>> {
        Box::pin(async move {
            let _ = self.calls.fetch_add(1, Ordering::SeqCst);
            let key = args["key"].as_str().unwrap_or_default().to_owned();
            match args["fail"].as_str() {
                Some("denied") => Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "not your issue",
                )),
                Some("down") => Err(Error::new(
                    ErrorKind::Unavailable,
                    "database restarting",
                )),
                _ => Ok(vec![Change::put(
                    self.space,
                    "issues",
                    &key,
                    json!({
                        "value": args["value"],
                        "by": context.client.as_str()
                    }),
                )]),
            }
        })
    }
}

struct World {
    sync: Sync<MemoryStore, Shared>,
    access: Shared,
    mutator: Arc<SetIssue>,
    space: SpaceId,
    user: UserId,
}

fn world() -> World {
    let space = SpaceId::generate();
    let access = Shared(Arc::new(Access::default()));
    let _ = access.allowed.lock().unwrap().insert(space);
    let mutator = Arc::new(SetIssue {
        space,
        calls: AtomicUsize::new(0),
    });
    let mut mutators = Mutators::new();
    mutators.register("issues.set", mutator.clone()).unwrap();
    World {
        sync: Sync::new(MemoryStore::new(), access.clone(), mutators),
        access,
        mutator,
        space,
        user: UserId::generate(),
    }
}

fn client() -> ClientId {
    ClientId::parse("device-1").unwrap()
}

fn mutation(id: u64, args: Value) -> ClientMutation {
    ClientMutation {
        id,
        name: "issues.set".to_owned(),
        args,
        created_at: 0,
    }
}

fn push(mutations: Vec<ClientMutation>) -> PushRequest {
    PushRequest {
        client_id: client(),
        mutations,
    }
}

fn pull(cursor: &str) -> PullRequest {
    PullRequest {
        client_id: client(),
        cursor: cursor.to_owned(),
    }
}

fn keys(patches: &[Patch]) -> Vec<String> {
    patches
        .iter()
        .map(|patch| match patch {
            Patch::Put { key, .. } => format!("put {key}"),
            Patch::Del { key, .. } => format!("del {key}"),
        })
        .collect()
}

#[tokio::test]
async fn applies_each_mutation_once() {
    let w = world();
    let batch = push(vec![
        mutation(1, json!({"key": "ENG-1", "value": 1})),
        mutation(2, json!({"key": "ENG-2"})),
    ]);
    let first = w
        .sync
        .push(w.user, RequestId::generate(), batch.clone())
        .await
        .unwrap();
    assert_eq!(first.last_mutation_id, 2);
    assert!(first.rejected.is_empty());
    let again = w
        .sync
        .push(w.user, RequestId::generate(), batch)
        .await
        .unwrap();
    assert_eq!(again.last_mutation_id, 2);
    assert_eq!(w.mutator.calls.load(Ordering::SeqCst), 2);

    let pulled = w.sync.pull(w.user, pull("")).await.unwrap();
    assert!(pulled.reset);
    assert_eq!(pulled.last_mutation_id, 2);
    assert_eq!(keys(&pulled.patches), ["put ENG-1", "put ENG-2"]);
    assert!(!pulled.more);
}

#[tokio::test]
async fn rejects_and_continues_or_stops_for_a_retry() {
    let w = world();
    let response = w
        .sync
        .push(
            w.user,
            RequestId::generate(),
            push(vec![
                mutation(1, json!({"key": "a", "fail": "denied"})),
                ClientMutation {
                    id: 2,
                    name: "nope".to_owned(),
                    args: json!({}),
                    created_at: 0,
                },
                mutation(3, json!({"key": "b"})),
                mutation(4, json!({"key": "c", "fail": "down"})),
                mutation(5, json!({"key": "d"})),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(response.last_mutation_id, 3);
    let codes: Vec<_> = response
        .rejected
        .iter()
        .map(|r| (r.id, r.code.as_str()))
        .collect();
    assert_eq!(codes, [(1, "permission_denied"), (2, "invalid_input")]);
    let retried = w
        .sync
        .push(
            w.user,
            RequestId::generate(),
            push(vec![
                mutation(4, json!({"key": "c"})),
                mutation(5, json!({"key": "d"})),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(retried.last_mutation_id, 5);
}

#[tokio::test]
async fn refuses_malformed_batches_and_foreign_clients() {
    let w = world();
    let out_of_order =
        push(vec![mutation(2, json!({})), mutation(1, json!({}))]);
    let err = w
        .sync
        .push(w.user, RequestId::generate(), out_of_order)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let huge = push(
        (1..=u64::try_from(MAX_PUSH).unwrap() + 1)
            .map(|id| mutation(id, json!({})))
            .collect(),
    );
    assert_eq!(
        w.sync
            .push(w.user, RequestId::generate(), huge)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );

    let _ = w
        .sync
        .push(
            w.user,
            RequestId::generate(),
            push(vec![mutation(1, json!({"key": "a"}))]),
        )
        .await
        .unwrap();
    let stranger = UserId::generate();
    let err = w
        .sync
        .push(
            stranger,
            RequestId::generate(),
            push(vec![mutation(2, json!({}))]),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    assert_eq!(
        w.sync.pull(stranger, pull("")).await.unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
}

#[tokio::test]
async fn pulls_incrementally_and_pages() {
    let w = world();
    let first = w.sync.pull(w.user, pull("")).await.unwrap();
    assert!(first.patches.is_empty());
    let changes: Vec<Change> = (0..=PULL_PAGE)
        .map(|n| Change::put(w.space, "issues", &format!("K-{n:05}"), json!(n)))
        .collect();
    w.sync.publish(changes).await.unwrap();
    w.sync.publish(vec![]).await.unwrap();

    let page = w.sync.pull(w.user, pull(&first.cursor)).await.unwrap();
    assert!(!page.reset);
    assert!(page.more);
    assert_eq!(page.patches.len(), PULL_PAGE);
    let rest = w.sync.pull(w.user, pull(&page.cursor)).await.unwrap();
    assert!(!rest.more);
    assert_eq!(keys(&rest.patches), [format!("put K-{PULL_PAGE:05}")]);

    w.sync
        .publish(vec![Change::delete(w.space, "issues", "K-00000")])
        .await
        .unwrap();
    let delta = w.sync.pull(w.user, pull(&rest.cursor)).await.unwrap();
    assert_eq!(keys(&delta.patches), ["del K-00000"]);
    let idle = w.sync.pull(w.user, pull(&delta.cursor)).await.unwrap();
    assert!(idle.patches.is_empty());
    assert_eq!(idle.cursor, delta.cursor);
}

#[tokio::test]
async fn hides_what_the_user_cannot_read_and_resets_on_permission_changes() {
    let w = world();
    let secret = SpaceId::generate();
    let object = vellum_kernel::ObjectId::generate();
    w.sync
        .publish(vec![
            Change::put(w.space, "issues", "open", json!(1)),
            Change::put(secret, "issues", "secret", json!(2)),
            Change::put(w.space, "pages", "narrowed", json!(3))
                .governed_by(object),
        ])
        .await
        .unwrap();
    let pulled = w.sync.pull(w.user, pull("")).await.unwrap();
    assert_eq!(keys(&pulled.patches), ["put open"]);

    let _ = w.access.allowed.lock().unwrap().insert(secret);
    let _ = w.access.epoch.fetch_add(1, Ordering::SeqCst);
    let after = w.sync.pull(w.user, pull(&pulled.cursor)).await.unwrap();
    assert!(after.reset);
    assert_eq!(keys(&after.patches), ["put open", "put secret"]);
}

#[tokio::test]
async fn resets_after_a_rebuild_a_prune_or_a_bad_cursor() {
    let w = world();
    w.sync
        .publish(vec![
            Change::put(w.space, "t", "a", json!(1)),
            Change::delete(w.space, "t", "b"),
        ])
        .await
        .unwrap();
    let pulled = w.sync.pull(w.user, pull("")).await.unwrap();
    assert_eq!(
        keys(&pulled.patches),
        ["put a"],
        "a snapshot skips tombstones"
    );

    w.sync.store().prune_tombstones(10);
    assert!(
        w.sync
            .pull(w.user, pull(&pulled.cursor))
            .await
            .unwrap()
            .reset
    );
    assert!(w.sync.pull(w.user, pull("v1.garbage")).await.unwrap().reset);

    let rebuilt = Sync::new(
        MemoryStore::with_epoch(2),
        w.access.clone(),
        Mutators::new(),
    );
    let after = rebuilt.pull(w.user, pull(&pulled.cursor)).await.unwrap();
    assert!(after.reset);
    assert!(after.cursor.starts_with("v1.2."));
}

#[tokio::test]
async fn mutator_names_are_unique() {
    let w = world();
    let mut mutators = Mutators::new();
    mutators.register("issues.set", w.mutator.clone()).unwrap();
    let err = mutators.register("issues.set", w.mutator).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(mutators.names(), ["issues.set"]);
    assert!(format!("{mutators:?}").contains("issues.set"));
}

#[tokio::test]
async fn memory_store_conforms() {
    check_store(MemoryStore::new).await.unwrap();
}

/// `slow.set` takes a while, like a mutator waiting on a busy database.
struct Slow {
    space: SpaceId,
    calls: AtomicUsize,
}

impl ServerMutator for Slow {
    fn apply<'a>(
        &'a self,
        _context: &'a MutationContext,
        _args: Value,
    ) -> BoxFuture<'a, Result<Vec<Change>, Error>> {
        Box::pin(async move {
            let calls = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            Ok(vec![Change::put(self.space, "counters", "c", json!(calls))])
        })
    }
}

/// A device whose push times out retries it while the server is still
/// working on the first one. The retry must not run the mutations again.
#[tokio::test]
async fn a_push_retried_while_the_first_still_runs_applies_nothing_twice() {
    let slow = Arc::new(Slow {
        space: SpaceId::generate(),
        calls: AtomicUsize::new(0),
    });
    let mut mutators = Mutators::new();
    mutators.register("slow.set", slow.clone()).unwrap();
    let sync = Sync::new(
        MemoryStore::new(),
        Shared(Arc::new(Access::default())),
        mutators,
    );
    let user = UserId::generate();
    let batch = PushRequest {
        client_id: client(),
        mutations: vec![ClientMutation {
            id: 1,
            name: "slow.set".to_owned(),
            args: json!({}),
            created_at: 0,
        }],
    };
    let both: [_; 2] = tokio::join!(
        sync.push(user, RequestId::generate(), batch.clone()),
        sync.push(user, RequestId::generate(), batch.clone()),
    )
    .into();
    assert_eq!(slow.calls.load(Ordering::SeqCst), 1, "applied twice");
    for result in both {
        match result {
            Ok(response) => assert_eq!(response.last_mutation_id, 1),
            Err(err) => assert!(err.kind().is_retryable(), "{err}"),
        }
    }
    // Once the first push is done, the retry goes through and skips it.
    let later = sync.push(user, RequestId::generate(), batch).await.unwrap();
    assert_eq!(later.last_mutation_id, 1);
    assert_eq!(slow.calls.load(Ordering::SeqCst), 1);
}

/// The log stores mutation IDs as signed 64-bit numbers, so a larger ID
/// is refused before anything runs, not after the mutator has applied it.
#[tokio::test]
async fn mutation_ids_beyond_the_store_range_are_refused_up_front() {
    let w = world();
    let too_big = u64::try_from(i64::MAX).unwrap() + 1;
    let err = w
        .sync
        .push(
            w.user,
            RequestId::generate(),
            push(vec![mutation(too_big, json!({"key": "a"}))]),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert_eq!(w.mutator.calls.load(Ordering::SeqCst), 0);
}

/// A cursor past the end of the log comes from a restored backup or a
/// corrupted device store. Resuming from it would skip every change up
/// to that position, so the device resets instead.
#[tokio::test]
async fn a_cursor_ahead_of_the_log_resets() {
    let w = world();
    w.sync
        .publish(vec![Change::put(w.space, "t", "a", json!(1))])
        .await
        .unwrap();
    let pulled = w.sync.pull(w.user, pull("")).await.unwrap();
    let epochs = pulled.cursor.rsplit_once('.').unwrap().0.to_owned();
    for seq in ["2", "500", &u64::MAX.to_string()] {
        let ahead = w
            .sync
            .pull(w.user, pull(&format!("{epochs}.{seq}")))
            .await
            .unwrap();
        assert!(ahead.reset, "cursor at {seq}");
        assert_eq!(keys(&ahead.patches), ["put a"]);
        assert_eq!(ahead.cursor, pulled.cursor);
    }
}
