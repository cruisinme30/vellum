// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Named groups through their operations (ADR 0057, D273-D275).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::significant_drop_tightening,
    clippy::indexing_slicing,
    reason = "tests fail loudly"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vellum_contracts::{
    BoxFuture, CredentialKind, Operation, Principal, RequestContext, Scope, ScopeSet, Service,
};
use vellum_identity::{GroupMembers, GroupsService, Identity, IdentityConfig, Store};
use vellum_kernel::{Error, ErrorKind, ObjectId, RequestId, Timestamp, UserId};

#[derive(Debug, Default)]
struct Members(Mutex<BTreeMap<ObjectId, BTreeSet<UserId>>>);

impl GroupMembers for Members {
    fn members(&self, group: ObjectId) -> BoxFuture<'_, Result<BTreeSet<UserId>, Error>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .get(&group)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(found) })
    }

    fn change<'a>(
        &'a self,
        group: ObjectId,
        add: &'a [UserId],
        remove: &'a [UserId],
    ) -> BoxFuture<'a, Result<(), Error>> {
        let mut all = self.0.lock().unwrap();
        let set = all.entry(group).or_default();
        set.extend(add.iter().copied());
        for user in remove {
            let _ = set.remove(user);
        }
        Box::pin(async { Ok(()) })
    }
}

fn ctx(scopes: &[Scope]) -> RequestContext {
    RequestContext {
        request_id: RequestId::generate(),
        principal: Principal::user(
            UserId::generate(),
            CredentialKind::Session,
            ScopeSet::from_scopes(scopes.iter().copied()),
            Timestamp::now(),
        ),
        deadline: Instant::now() + Duration::from_secs(10),
        traceparent: None,
    }
}

struct World {
    identity: Arc<Identity>,
    members: Arc<Members>,
    ops: Vec<Operation>,
}

impl World {
    fn new() -> Self {
        let identity = Arc::new(Identity::new(
            IdentityConfig::new("https://vellum.northstar.dev".parse().unwrap()),
            Store::memory(),
        ));
        let members = Arc::new(Members::default());
        let service = Arc::new(GroupsService::new(
            Arc::clone(&identity),
            Some(Arc::clone(&members) as Arc<dyn GroupMembers>),
        ));
        Self {
            identity,
            members,
            ops: service.operations(),
        }
    }

    async fn call(&self, id: &str, input: Value) -> Result<Value, Error> {
        let op = self.ops.iter().find(|o| o.id() == id).expect("declared");
        op.call(ctx(&[Scope::Admin]), input).await
    }
}

#[tokio::test]
async fn admins_name_groups_and_manage_their_members() {
    let w = World::new();
    let priya = w
        .identity
        .create_user("priya@northstar.dev", "Priya Shah", None, false)
        .unwrap();
    let marco = w
        .identity
        .create_user("marco@northstar.dev", "Marco Ruiz", None, false)
        .unwrap();

    let sre = w
        .call(
            "groups.create",
            json!({"name": " SRE ", "description": "On call"}),
        )
        .await
        .unwrap();
    let id = sre["id"].as_str().unwrap().to_owned();
    assert_eq!(sre["name"], "SRE");
    let clash = w
        .call("groups.create", json!({"name": "sre"}))
        .await
        .unwrap_err();
    assert_eq!(clash.kind(), ErrorKind::Conflict);

    let after = w
        .call(
            "groups.change_members",
            json!({"id": id, "add": [priya.id, marco.id]}),
        )
        .await
        .unwrap();
    let names: Vec<&str> = after["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Marco Ruiz", "Priya Shah"]);

    // Blocked people cannot join.
    let _ = w
        .identity
        .update_user(marco.id, None, None, Some(true))
        .unwrap();
    let blocked = w
        .call(
            "groups.change_members",
            json!({"id": id, "add": [marco.id]}),
        )
        .await
        .unwrap_err();
    assert_eq!(blocked.kind(), ErrorKind::InvalidInput);

    let renamed = w
        .call(
            "groups.update",
            json!({"id": id, "name": "Site reliability"}),
        )
        .await
        .unwrap();
    assert_eq!(renamed["description"], "On call");

    // Deleting a group empties it first, so access through it ends.
    let _ = w.call("groups.delete", json!({"id": id})).await.unwrap();
    let group: ObjectId = id.parse().unwrap();
    assert!(w.members.0.lock().unwrap()[&group].is_empty());
    let gone = w
        .call("groups.members", json!({"id": id}))
        .await
        .unwrap_err();
    assert_eq!(gone.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn people_pickers_find_groups_by_name() {
    let w = World::new();
    for name in ["Design", "SRE", "Site leads"] {
        let _ = w.identity.create_group(name, "").unwrap();
    }
    let search = w.ops.iter().find(|o| o.id() == "groups.search").unwrap();
    assert_eq!(search.scope(), Scope::Read, "anyone signed in can search");
    assert!(search.fresh_auth_window().is_none());
    let found = search
        .call(ctx(&[Scope::Read]), json!({"q": "s"}))
        .await
        .unwrap();
    let entries = found["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Site leads", "SRE", "Design"],
        "prefix matches first"
    );
    assert_eq!(entries[0]["kind"], "group");
    assert!(
        entries[0]["principal"]
            .as_str()
            .unwrap()
            .starts_with("group:")
    );
    assert_eq!(entries[1]["detail"], "Group");

    let anonymous = RequestContext {
        principal: Principal::anonymous(),
        ..ctx(&[])
    };
    assert!(search.call(anonymous, json!({})).await.is_err());

    // The admin operations all need a recent sign-in.
    for op in w.ops.iter().filter(|o| o.id() != "groups.search") {
        assert_eq!(op.scope(), Scope::Admin, "{}", op.id());
        assert!(op.fresh_auth_window().is_some(), "{}", op.id());
    }
}
