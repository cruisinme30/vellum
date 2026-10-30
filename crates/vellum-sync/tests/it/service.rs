// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `sync.push` and `sync.pull` through the gateway seam, over the SQLite
//! store, with the JSON a client sends.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helpers fail loudly"
)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vellum_contracts::{
    CredentialKind, Operation, Principal, Readiness, RequestContext, Scope, ScopeSet, Service,
};
use vellum_db::{Index, IndexConfig};
use vellum_kernel::{Error, ErrorKind, RequestId, SpaceId, Timestamp, UserId};
use vellum_sync::ports::BoxFuture;
use vellum_sync::{
    AccessKey, Change, MutationContext, Mutators, ReadAccess, ServerMutator, SqlStore, Sync,
};

/// Everyone reads everything; the tests are about the seam, not authz.
struct Open;

impl ReadAccess for Open {
    async fn epoch(&self, _user: UserId) -> Result<u64, Error> {
        Ok(1)
    }

    async fn readable(
        &self,
        _user: UserId,
        keys: &[AccessKey],
    ) -> Result<HashSet<AccessKey>, Error> {
        Ok(keys.iter().copied().collect())
    }
}

/// `issues.set {key, title}` writes one record in a fixed space.
struct SetIssue(SpaceId);

impl ServerMutator for SetIssue {
    fn apply<'a>(
        &'a self,
        _context: &'a MutationContext,
        args: Value,
    ) -> BoxFuture<'a, Result<Vec<Change>, Error>> {
        Box::pin(async move {
            let key = args["key"].as_str().unwrap_or_default().to_owned();
            Ok(vec![Change::put(self.0, "issues", &key, args)])
        })
    }
}

fn ctx(principal: Principal) -> RequestContext {
    RequestContext {
        request_id: RequestId::generate(),
        principal,
        deadline: Instant::now() + Duration::from_secs(5),
        traceparent: None,
    }
}

fn person(user: UserId, scopes: &[Scope]) -> Principal {
    Principal::user(
        user,
        CredentialKind::Session,
        ScopeSet::from_scopes(scopes.iter().copied()),
        Timestamp::now(),
    )
}

async fn operations(dir: &tempfile::TempDir) -> (Arc<Sync<SqlStore, Open>>, Vec<Operation>) {
    let index = Index::open(&IndexConfig::sqlite(dir.path().join("i.sqlite"), None))
        .await
        .unwrap();
    let mut mutators = Mutators::new();
    mutators
        .register("issues.set", Arc::new(SetIssue(SpaceId::generate())))
        .unwrap();
    let engine = Arc::new(Sync::new(SqlStore::new(index), Open, mutators));
    let ops = Arc::clone(&engine).operations();
    (engine, ops)
}

fn find<'a>(ops: &'a [Operation], id: &str) -> &'a Operation {
    ops.iter().find(|op| op.id() == id).unwrap()
}

#[tokio::test]
async fn operations_are_declared_as_the_contract_says() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, ops) = operations(&dir).await;
    assert_eq!(engine.name(), "sync");
    let routes: Vec<_> = ops
        .iter()
        .map(|op| (op.id(), op.method().as_str(), op.path(), op.scope()))
        .collect();
    assert_eq!(
        routes,
        vec![
            ("sync.push", "POST", "/v1/sync/push", Scope::WriteDirect),
            ("sync.pull", "GET", "/v1/sync/pull", Scope::Read),
        ]
    );
    for op in &ops {
        op.validate("sync").unwrap();
    }
    assert_eq!(engine.readiness().await, Readiness::Ready);
}

#[tokio::test]
async fn a_push_is_pulled_back_with_client_field_names() {
    let dir = tempfile::tempdir().unwrap();
    let (_engine, ops) = operations(&dir).await;
    let ada = person(UserId::generate(), &[Scope::Read, Scope::WriteDirect]);

    let pushed = find(&ops, "sync.push")
        .call(
            ctx(ada.clone()),
            json!({
                "clientId": "laptop-1",
                "mutations": [{
                    "id": 1,
                    "name": "issues.set",
                    "args": {"key": "ENG-1", "title": "Ship sync"},
                    "createdAt": 1_761_730_000_000_i64
                }]
            }),
        )
        .await
        .unwrap();
    assert_eq!(pushed, json!({"lastMutationId": 1, "rejected": []}));

    let pulled = find(&ops, "sync.pull")
        .call(ctx(ada), json!({"clientId": "laptop-1"}))
        .await
        .unwrap();
    assert_eq!(pulled["lastMutationId"], json!(1));
    assert_eq!(pulled["reset"], json!(true));
    assert_eq!(
        pulled["patches"],
        json!([{
            "op": "put",
            "collection": "issues",
            "key": "ENG-1",
            "value": {"key": "ENG-1", "title": "Ship sync"}
        }])
    );
    assert!(pulled["cursor"].as_str().is_some_and(|c| !c.is_empty()));
}

#[tokio::test]
async fn callers_need_a_person_and_the_scope() {
    let dir = tempfile::tempdir().unwrap();
    let (_engine, ops) = operations(&dir).await;
    let push = json!({"clientId": "laptop-1", "mutations": []});
    let pull = json!({"clientId": "laptop-1"});

    let anonymous = find(&ops, "sync.pull")
        .call(ctx(Principal::anonymous()), pull.clone())
        .await
        .unwrap_err();
    assert_eq!(anonymous.kind(), ErrorKind::Unauthenticated);

    let reader = person(UserId::generate(), &[Scope::Read]);
    let refused = find(&ops, "sync.push")
        .call(ctx(reader.clone()), push)
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::PermissionDenied);
    let _ = find(&ops, "sync.pull")
        .call(ctx(reader), pull)
        .await
        .unwrap();

    let bad = find(&ops, "sync.pull")
        .call(
            ctx(person(UserId::generate(), &[Scope::Read])),
            json!({"clientId": "no spaces"}),
        )
        .await
        .unwrap_err();
    assert_eq!(bad.kind(), ErrorKind::InvalidInput);
}
