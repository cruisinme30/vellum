// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SCIM 2.0 provisioning (issue #523, ADR 0069 D294), over HTTP as an
//! identity provider calls it, including token misuse.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::significant_drop_tightening,
    reason = "test helpers fail loudly"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use url::Url;
use vellum_contracts::{BoxFuture, Scope};
use vellum_identity::{GroupMembers, Identity, IdentityConfig, Store, scim};
use vellum_kernel::{Error, ErrorKind, ObjectId, UserId};

const PASSWORD: &str = "correct horse battery staple";

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
        {
            let mut all = self.0.lock().unwrap();
            let set = all.entry(group).or_default();
            set.extend(add.iter().copied());
            for user in remove {
                let _ = set.remove(user);
            }
        }
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    identity: Arc<Identity>,
    members: Arc<Members>,
    router: Router,
    token: String,
}

fn fixture() -> Fixture {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    let identity = Arc::new(Identity::new(IdentityConfig::new(url), Store::memory()));
    let _ = identity
        .setup("priya@northstar.dev", "Priya", PASSWORD)
        .unwrap();
    let token = identity.rotate_scim_token().unwrap().token;
    let members = Arc::new(Members::default());
    let router = scim::router(
        Arc::clone(&identity),
        Some(Arc::clone(&members) as Arc<dyn GroupMembers>),
    );
    Fixture {
        identity,
        members,
        router,
        token,
    }
}

impl Fixture {
    async fn call_with(
        &self,
        token: Option<&str>,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/scim+json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        if status != StatusCode::NO_CONTENT {
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/scim+json"
            );
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.call_with(Some(&self.token.clone()), method, path, body)
            .await
    }
}

fn sam() -> Value {
    json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
        "userName": "Sam@Northstar.dev",
        "externalId": "00u-sam",
        "name": { "givenName": "Sam", "familyName": "Okafor" },
        "emails": [{ "value": "sam@northstar.dev", "primary": true, "type": "work" }],
        "active": true,
        "roles": [{ "value": "admin" }],
    })
}

#[tokio::test]
async fn only_the_current_scim_token_works() {
    let f = fixture();
    let (status, body) = f.call_with(None, "GET", "/Users", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body["schemas"][0],
        "urn:ietf:params:scim:api:messages:2.0:Error"
    );
    // A forged token of the right shape, and an API token, are refused.
    let forged = format!("vsc_{}", "A".repeat(43));
    let priya = f.identity.users()[0].id;
    let api = f
        .identity
        .create_api_token(priya, "CI", &[Scope::Admin], None)
        .unwrap()
        .0;
    for token in [forged.as_str(), api.as_str(), "", "vsc_short"] {
        let (status, _) = f.call_with(Some(token), "GET", "/Users", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{token}");
    }
    // The SCIM token is only a SCIM token.
    assert!(f.identity.bearer(&f.token).is_err());
    // Rotating ends the old token; turning SCIM off ends every token.
    let (status, _) = f.call("GET", "/ServiceProviderConfig", None).await;
    assert_eq!(status, StatusCode::OK);
    let new = f.identity.rotate_scim_token().unwrap();
    assert!(new.scim.enabled);
    let (status, _) = f.call("GET", "/Users", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = f.call_with(Some(&new.token), "GET", "/Users", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!f.identity.revoke_scim_token().unwrap().enabled);
    let (status, _) = f.call_with(Some(&new.token), "GET", "/Users", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The token is never shown again.
    let shown = serde_json::to_string(&f.identity.sign_in_settings()).unwrap();
    assert!(!shown.contains(&new.token));
}

#[tokio::test]
async fn providers_create_find_change_and_block_people() {
    let f = fixture();
    let (status, created) = f.call("POST", "/Users", Some(sam())).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["userName"], "sam@northstar.dev");
    assert_eq!(created["displayName"], "Sam Okafor");
    assert_eq!(created["externalId"], "00u-sam");
    assert_eq!(created["active"], true);
    let user: UserId = id.parse().unwrap();
    let view = f.identity.user(user).unwrap();
    assert!(!view.admin, "provisioning never makes admins");
    assert!(!view.has_password);

    // Creating it again is a conflict; the provider then finds it.
    let (status, error) = f.call("POST", "/Users", Some(sam())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["scimType"], "uniqueness");
    let (status, found) = f
        .call(
            "GET",
            "/Users?filter=userName%20eq%20%22SAM%40northstar.dev%22",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found["totalResults"], 1);
    assert_eq!(found["Resources"][0]["id"], id.as_str());
    let (_, by_external) = f
        .call("GET", "/Users?filter=externalId%20eq%20%2200u-sam%22", None)
        .await;
    assert_eq!(by_external["totalResults"], 1);
    let (status, _) = f
        .call("GET", "/Users?filter=userName%20sw%20%22s%22", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A session and a token, to see blocking end them.
    let token = f
        .identity
        .create_api_token(user, "sync", &[Scope::Read], None)
        .unwrap()
        .0;
    assert!(f.identity.bearer(&token).is_ok());

    // Entra ID sends booleans as strings.
    let patch = json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [
            { "op": "Replace", "path": "displayName", "value": "Samuel Okafor" },
            { "op": "Replace", "path": "active", "value": "False" },
            { "op": "Add", "path": "title", "value": "Engineer" },
        ],
    });
    let (status, patched) = f.call("PATCH", &format!("/Users/{id}"), Some(patch)).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["active"], false);
    assert_eq!(patched["displayName"], "Samuel Okafor");
    assert!(f.identity.user(user).unwrap().disabled);
    assert!(f.identity.bearer(&token).is_err(), "blocking ends tokens");

    // Okta replaces the whole resource to reactivate.
    let mut back = sam();
    back["active"] = json!(true);
    let (status, replaced) = f.call("PUT", &format!("/Users/{id}"), Some(back)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replaced["active"], true);

    // Deleting blocks the account and keeps it.
    let (status, _) = f.call("DELETE", &format!("/Users/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let view = f.identity.user(user).unwrap();
    assert!(view.disabled);
    let (status, _) = f.call("GET", "/Users/not-an-id", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn break_glass_admins_are_out_of_reach() {
    let f = fixture();
    let priya = f.identity.users()[0].clone();
    let _ = f.identity.set_break_glass(priya.id, true).unwrap();
    let off = json!({ "Operations": [{ "op": "replace", "path": "active", "value": false }] });
    let (status, _) = f
        .call("PATCH", &format!("/Users/{}", priya.id), Some(off.clone()))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f
        .call("DELETE", &format!("/Users/{}", priya.id), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!f.identity.user(priya.id).unwrap().disabled);
    // Without break-glass, the last admin is still kept.
    let _ = f.identity.set_break_glass(priya.id, false).unwrap();
    let (status, _) = f
        .call("PATCH", &format!("/Users/{}", priya.id), Some(off))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn groups_are_admin_groups_with_engine_members() {
    let f = fixture();
    let (_, sam) = f.call("POST", "/Users", Some(sam())).await;
    let sam_id = sam["id"].as_str().unwrap().to_owned();
    let priya = f
        .identity
        .users()
        .into_iter()
        .find(|u| u.email == "priya@northstar.dev")
        .unwrap();

    let (status, group) = f
        .call(
            "POST",
            "/Groups",
            Some(json!({ "displayName": "Engineering", "externalId": "g-eng", "members": [{ "value": sam_id }] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let gid = group["id"].as_str().unwrap().to_owned();
    let object: ObjectId = gid.parse().unwrap();
    assert_eq!(group["members"][0]["value"], sam_id.as_str());
    assert_eq!(f.identity.group(object).unwrap().name, "Engineering");

    // Names stay unique ignoring case.
    let (status, _) = f
        .call(
            "POST",
            "/Groups",
            Some(json!({ "displayName": "engineering" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Add Priya, remove Sam (Entra's filter path), rename.
    let patch = json!({ "Operations": [
        { "op": "add", "path": "members", "value": [{ "value": priya.id.to_string() }] },
        { "op": "remove", "path": format!("members[value eq \"{sam_id}\"]") },
        { "op": "replace", "value": { "displayName": "Platform" } },
    ]});
    let (status, patched) = f
        .call("PATCH", &format!("/Groups/{gid}"), Some(patch))
        .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["displayName"], "Platform");
    let members: BTreeSet<UserId> = f.members.0.lock().unwrap()[&object].clone();
    assert_eq!(members, BTreeSet::from([priya.id]));

    // Unknown people cannot be added.
    let bad = json!({ "Operations": [{ "op": "add", "path": "members", "value": [{ "value": UserId::generate().to_string() }] }] });
    let (status, _) = f.call("PATCH", &format!("/Groups/{gid}"), Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, listed) = f
        .call(
            "GET",
            "/Groups?filter=displayName%20eq%20%22platform%22&excludedAttributes=members",
            None,
        )
        .await;
    assert_eq!(listed["totalResults"], 1);
    assert!(listed["Resources"][0].get("members").is_none());

    // Deleting removes the members first, then the group.
    let (status, _) = f.call("DELETE", &format!("/Groups/{gid}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(f.members.0.lock().unwrap()[&object].is_empty());
    assert_eq!(
        f.identity.group(object).unwrap_err().kind(),
        ErrorKind::NotFound
    );
}
