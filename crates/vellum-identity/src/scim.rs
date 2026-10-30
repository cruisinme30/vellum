// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SCIM 2.0 provisioning (RFC 7643, RFC 7644; ADR 0069, D294).
//!
//! An identity provider creates, changes and blocks accounts and groups
//! at `/scim/v2/Users` and `/scim/v2/Groups`, with the one `vsc_` bearer
//! token an admin makes in Admin → Sign-in → Provisioning. The token is
//! stored as a hash and works only here; no other credential works here.
//!
//! - Accounts it creates have no password and are never admins.
//!   `active: false` or a delete blocks the account, which ends its
//!   sessions and tokens; its content stays where it is.
//! - Break-glass admins are managed only in Vellum, and an admin's
//!   address is changed only in Vellum (RT-044).
//! - Groups are the admin groups (names unique ignoring case); members
//!   change through the permission engine ([`GroupMembers`]).
//! - Filters: `<attribute> eq "<value>"` on `userName`, `externalId`,
//!   `emails.value`, `displayName` and `id`.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;
use vellum_kernel::{Error, ErrorKind, ObjectId, Timestamp, UserId};

use crate::groups::GroupMembers;
use crate::identity::{
    BREAK_GLASS_EMAIL, Identity, check_name, normalise_email,
};
use crate::secret::{self, Kind};
use crate::store::{Group, ScimToken, User};

/// Where the SCIM endpoints are mounted.
pub const SCIM_PREFIX: &str = "/scim/v2";
const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
const CONFIG_SCHEMA: &str =
    "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";
const SCIM_JSON: &str = "application/scim+json";
/// The most resources one list returns.
const MAX_PAGE: usize = 200;
/// The longest external ID kept.
const MAX_EXTERNAL_ID: usize = 255;

/// Provisioning as Admin → Sign-in shows it. The token is never shown
/// again after it is made.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScimView {
    /// Whether a token is set, so providers can provision.
    pub enabled: bool,
    /// When the current token was made.
    pub token_created_at: Option<Timestamp>,
    /// The SCIM base URL to give the provider.
    pub base_url: String,
}

/// A new SCIM token, shown once.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScimTokenCreated {
    /// The token (`vsc_…`), for the provider's settings.
    pub token: String,
    /// Provisioning as it now is.
    pub scim: ScimView,
}

fn token_hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(secret::hash(token))
}

impl Identity {
    /// Whether provisioning is on, and where.
    #[must_use]
    pub fn scim_status(&self) -> ScimView {
        let token = self.store().read(|s| s.sign_in.scim.clone());
        ScimView {
            enabled: token.is_some(),
            token_created_at: token.map(|t| t.created_at),
            base_url: format!("{}{SCIM_PREFIX}", self.config().base()),
        }
    }

    /// Makes a new SCIM token, turning provisioning on; the old token
    /// stops working at once.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the random source fails.
    pub fn rotate_scim_token(&self) -> Result<ScimTokenCreated, Error> {
        let token = secret::generate(Kind::ScimToken)?;
        let record = ScimToken {
            hash: token_hash(&token),
            created_at: Timestamp::now(),
        };
        self.store().write(|s| s.sign_in.scim = Some(record))?;
        tracing::info!(
            event = "identity.scim_token_rotated",
            "SCIM token made"
        );
        Ok(ScimTokenCreated {
            token,
            scim: self.scim_status(),
        })
    }

    /// Removes the SCIM token, turning provisioning off. Accounts and
    /// groups it made stay.
    ///
    /// # Errors
    ///
    /// Only if the store cannot be written.
    pub fn revoke_scim_token(&self) -> Result<ScimView, Error> {
        self.store().write(|s| s.sign_in.scim = None)?;
        tracing::info!(
            event = "identity.scim_token_revoked",
            "SCIM turned off"
        );
        Ok(self.scim_status())
    }

    /// Checks a presented SCIM token.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for anything but the current token.
    pub fn check_scim_token(&self, presented: &str) -> Result<(), Error> {
        let refused = || {
            Error::new(
                ErrorKind::Unauthenticated,
                "the SCIM token is wrong or was replaced",
            )
        };
        if !secret::well_formed(presented, Kind::ScimToken) {
            return Err(refused());
        }
        let stored = self
            .store()
            .read(|s| s.sign_in.scim.as_ref().map(|t| t.hash.clone()))
            .ok_or_else(refused)?;
        let presented = token_hash(presented);
        if bool::from(stored.as_bytes().ct_eq(presented.as_bytes())) {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

// --- Errors and responses ---

/// A SCIM error response (RFC 7644 §3.12).
#[derive(Debug)]
struct ScimError {
    status: StatusCode,
    scim_type: Option<&'static str>,
    detail: String,
}

impl ScimError {
    fn new(
        status: StatusCode,
        scim_type: Option<&'static str>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            status,
            scim_type,
            detail: detail.into(),
        }
    }

    fn bad(scim_type: &'static str, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, Some(scim_type), detail)
    }

    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, None, "no such resource")
    }
}

impl From<Error> for ScimError {
    fn from(e: Error) -> Self {
        let (status, scim_type) = match e.kind() {
            ErrorKind::NotFound => (StatusCode::NOT_FOUND, None),
            ErrorKind::Conflict => (StatusCode::CONFLICT, Some("uniqueness")),
            ErrorKind::InvalidInput => {
                (StatusCode::BAD_REQUEST, Some("invalidValue"))
            }
            ErrorKind::PermissionDenied => (StatusCode::FORBIDDEN, None),
            ErrorKind::Unauthenticated => (StatusCode::UNAUTHORIZED, None),
            ErrorKind::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, None),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, None),
        };
        Self::new(status, scim_type, e.message().to_owned())
    }
}

impl IntoResponse for ScimError {
    fn into_response(self) -> Response {
        let mut body = json!({
            "schemas": [ERROR_SCHEMA],
            "status": self.status.as_u16().to_string(),
            "detail": self.detail,
        });
        if let (Some(kind), Some(map)) = (self.scim_type, body.as_object_mut())
        {
            let _ = map.insert("scimType".to_owned(), Value::from(kind));
        }
        let mut response = scim_json(self.status, &body);
        if self.status == StatusCode::UNAUTHORIZED {
            let _ = response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}

type Answer = Result<Response, ScimError>;

fn scim_json(status: StatusCode, body: &Value) -> Response {
    let mut response = (status, body.to_string()).into_response();
    let headers = response.headers_mut();
    let _ = headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(SCIM_JSON));
    let _ = headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn created(location: &str, body: &Value) -> Response {
    let mut response = scim_json(StatusCode::CREATED, body);
    if let Ok(value) = HeaderValue::from_str(location) {
        let _ = response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

fn parse_body(body: &Bytes) -> Result<Value, ScimError> {
    serde_json::from_slice(body).map_err(|e| {
        ScimError::bad("invalidSyntax", format!("the body is not JSON: {e}"))
    })
}

// --- The router ---

#[derive(Clone)]
struct Scim {
    identity: Arc<Identity>,
    members: Option<Arc<dyn GroupMembers>>,
}

impl Scim {
    fn base(&self) -> String {
        format!("{}{SCIM_PREFIX}", self.identity.config().base())
    }

    fn port(&self) -> Result<&Arc<dyn GroupMembers>, ScimError> {
        self.members.as_ref().ok_or_else(|| {
            ScimError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                None,
                "group membership is unavailable on this server",
            )
        })
    }
}

async fn require_token(
    State(scim): State<Scim>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::trim);
    if presented.is_some_and(|t| scim.identity.check_scim_token(t).is_ok()) {
        return next.run(request).await;
    }
    tracing::info!(
        event = "identity.scim_refused",
        "a SCIM call had no valid token"
    );
    ScimError::new(
        StatusCode::UNAUTHORIZED,
        None,
        "send the SCIM token from Admin → Sign-in as a bearer token",
    )
    .into_response()
}

/// The SCIM endpoints, to mount cross-site at [`SCIM_PREFIX`]. `members`
/// is the permission engine's group membership; without it, group
/// members cannot be read or changed.
pub fn router(
    identity: Arc<Identity>,
    members: Option<Arc<dyn GroupMembers>>,
) -> Router {
    let state = Scim { identity, members };
    Router::new()
        .route("/ServiceProviderConfig", get(service_provider_config))
        .route("/Users", get(list_users).post(create_user))
        .route(
            "/Users/{id}",
            get(get_user)
                .put(replace_user)
                .patch(patch_user)
                .delete(delete_user),
        )
        .route("/Groups", get(list_groups).post(create_group))
        .route(
            "/Groups/{id}",
            get(get_group)
                .put(replace_group)
                .patch(patch_group)
                .delete(delete_group),
        )
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state)
}

async fn service_provider_config() -> Response {
    scim_json(
        StatusCode::OK,
        &json!({
            "schemas": [CONFIG_SCHEMA],
            "patch": { "supported": true },
            "bulk": {
                "supported": false,
                "maxOperations": 0,
                "maxPayloadSize": 0
            },
            "filter": { "supported": true, "maxResults": MAX_PAGE },
            "changePassword": { "supported": false },
            "sort": { "supported": false },
            "etag": { "supported": false },
            "authenticationSchemes": [{
                "type": "oauthbearertoken",
                "name": "Bearer token",
                "description": "The token made in Admin → Sign-in → \
                    Provisioning",
            }],
        }),
    )
}

// --- Filters and paging ---

#[derive(Debug, Deserialize)]
struct ListQuery {
    filter: Option<String>,
    #[serde(rename = "startIndex")]
    start_index: Option<usize>,
    count: Option<usize>,
    #[serde(rename = "excludedAttributes")]
    excluded_attributes: Option<String>,
}

/// `attribute eq "value"`, the one filter form providers send.
fn parse_filter(filter: &str) -> Result<(String, String), ScimError> {
    let unsupported = || {
        ScimError::bad(
            "invalidFilter",
            "only filters of the form attribute eq \"value\" are supported",
        )
    };
    let filter = filter.trim();
    let (attribute, rest) = filter
        .split_once(char::is_whitespace)
        .ok_or_else(unsupported)?;
    let rest = rest.trim_start();
    let (op, rest) = rest
        .split_once(char::is_whitespace)
        .ok_or_else(unsupported)?;
    if !op.eq_ignore_ascii_case("eq") {
        return Err(unsupported());
    }
    let quoted = rest.trim();
    let inner = quoted
        .strip_prefix('"')
        .and_then(|q| q.strip_suffix('"'))
        .ok_or_else(unsupported)?;
    let mut value = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => value.push(chars.next().ok_or_else(unsupported)?),
            '"' => return Err(unsupported()),
            c => value.push(c),
        }
    }
    Ok((attribute.to_lowercase(), value))
}

fn page(query: &ListQuery, resources: Vec<Value>) -> Response {
    let total = resources.len();
    let start = query.start_index.unwrap_or(1).max(1);
    let count = query.count.unwrap_or(MAX_PAGE).min(MAX_PAGE);
    let items: Vec<Value> = resources
        .into_iter()
        .skip(start.saturating_sub(1))
        .take(count)
        .collect();
    scim_json(
        StatusCode::OK,
        &json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": total,
            "startIndex": start,
            "itemsPerPage": items.len(),
            "Resources": items,
        }),
    )
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `true`, `false`, or the strings some providers send instead.
fn boolean(value: &Value) -> Result<bool, ScimError> {
    match value {
        Value::Bool(b) => Ok(*b),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Ok(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Ok(false),
        _ => Err(ScimError::bad(
            "invalidValue",
            "active must be true or false",
        )),
    }
}

fn external_id(value: &Value) -> Result<Option<String>, ScimError> {
    match value {
        Value::Null => Ok(None),
        other => {
            let id = text(other).ok_or_else(|| {
                ScimError::bad("invalidValue", "externalId must be a string")
            })?;
            if id.len() > MAX_EXTERNAL_ID {
                return Err(ScimError::bad(
                    "invalidValue",
                    "externalId is too long",
                ));
            }
            Ok(Some(id))
        }
    }
}

// --- Users ---

/// An account as SCIM sees it, before and after changes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Account {
    email: String,
    name: String,
    external_id: Option<String>,
    active: bool,
}

impl Account {
    fn of(user: &User) -> Self {
        Self {
            email: user.email.clone(),
            name: user.name.clone(),
            external_id: user.external_id.clone(),
            active: !user.disabled,
        }
    }

    /// Reads a whole User resource (create and replace).
    fn from_resource(body: &Value) -> Result<Self, ScimError> {
        let user_name =
            body.get("userName").and_then(text).ok_or_else(|| {
                ScimError::bad("invalidValue", "userName is required")
            })?;
        let emails = body.get("emails").and_then(Value::as_array);
        let email = emails
            .and_then(|list| {
                list.iter()
                    .find(|e| {
                        e.get("primary").and_then(Value::as_bool) == Some(true)
                    })
                    .or_else(|| list.first())
            })
            .and_then(|e| e.get("value"))
            .and_then(text)
            .unwrap_or_else(|| user_name.clone());
        let name = body
            .get("displayName")
            .and_then(text)
            .or_else(|| body.pointer("/name/formatted").and_then(text))
            .or_else(|| {
                joined(
                    body.pointer("/name/givenName"),
                    body.pointer("/name/familyName"),
                )
            })
            .unwrap_or_else(|| {
                email.split('@').next().unwrap_or_default().to_owned()
            });
        Ok(Self {
            email,
            name,
            external_id: body
                .get("externalId")
                .map(external_id)
                .transpose()?
                .flatten(),
            active: body
                .get("active")
                .map(boolean)
                .transpose()?
                .unwrap_or(true),
        })
    }
}

fn joined(given: Option<&Value>, family: Option<&Value>) -> Option<String> {
    let parts: Vec<String> = [given, family]
        .into_iter()
        .flatten()
        .filter_map(text)
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn user_resource(base: &str, user: &User) -> Value {
    let mut resource = json!({
        "schemas": [USER_SCHEMA],
        "id": user.id.to_string(),
        "userName": user.email,
        "displayName": user.name,
        "name": { "formatted": user.name },
        "emails": [{ "value": user.email, "type": "work", "primary": true }],
        "active": !user.disabled,
        "meta": {
            "resourceType": "User",
            "created": user.created_at.to_string(),
            "location": format!("{base}/Users/{}", user.id),
        },
    });
    if let (Some(id), Some(map)) = (&user.external_id, resource.as_object_mut())
    {
        let _ = map.insert("externalId".to_owned(), Value::from(id.clone()));
    }
    resource
}

fn find_user(identity: &Identity, id: &str) -> Result<User, ScimError> {
    let id: UserId = id.parse().map_err(|_| ScimError::not_found())?;
    identity
        .store()
        .read(|s| s.users.get(&id).cloned())
        .filter(|u| u.email != BREAK_GLASS_EMAIL)
        .ok_or_else(ScimError::not_found)
}

fn protected(user: &User) -> Result<(), ScimError> {
    if user.break_glass {
        return Err(ScimError::new(
            StatusCode::FORBIDDEN,
            None,
            "break-glass admins are managed in Vellum, not by provisioning",
        ));
    }
    Ok(())
}

/// Applies `next` to the account `user`, as one change.
fn apply_account(
    identity: &Identity,
    user: &User,
    next: &Account,
) -> Result<User, ScimError> {
    protected(user)?;
    let now = Account::of(user);
    let email = normalise_email(&next.email)?;
    let name = check_name(&next.name)?;
    // An admin's address decides which provider sign-ins reach the
    // account, so the provisioning token must not move it (RT-044).
    if user.admin && email != now.email {
        return Err(ScimError::new(
            StatusCode::FORBIDDEN,
            None,
            "an admin's address is changed in Vellum, not by provisioning",
        ));
    }
    if email != now.email || next.external_id != now.external_id {
        identity.store().write(|s| {
            if email != now.email
                && s.users
                    .values()
                    .any(|u| u.id != user.id && u.email == email)
            {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "another account has this address",
                ));
            }
            if let Some(found) = s.users.get_mut(&user.id) {
                found.email.clone_from(&email);
                found.external_id.clone_from(&next.external_id);
            }
            Ok(())
        })??;
    }
    let renamed = (name != now.name).then_some(name.as_str());
    let blocked = (next.active != now.active).then_some(!next.active);
    if renamed.is_some() || blocked.is_some() {
        let _ = identity.update_user(user.id, renamed, None, blocked)?;
    }
    if blocked == Some(true) {
        tracing::info!(
            event = "identity.scim_user_blocked",
            user = %user.id,
            "SCIM blocked an account"
        );
    }
    find_user(identity, &user.id.to_string())
}

async fn list_users(
    State(scim): State<Scim>,
    Query(query): Query<ListQuery>,
) -> Answer {
    let filter = query.filter.as_deref().map(parse_filter).transpose()?;
    let base = scim.base();
    let mut users: Vec<User> = scim.identity.store().read(|s| {
        s.users
            .values()
            .filter(|u| u.email != BREAK_GLASS_EMAIL)
            .cloned()
            .collect()
    });
    if let Some((attribute, value)) = &filter {
        let matches = |u: &User| match attribute.as_str() {
            "username" | "emails.value" | "emails" => {
                u.email.eq_ignore_ascii_case(value)
            }
            "externalid" => u.external_id.as_deref() == Some(value.as_str()),
            "id" => u.id.to_string() == *value,
            _ => false,
        };
        if !matches!(
            attribute.as_str(),
            "username" | "emails.value" | "emails" | "externalid" | "id"
        ) {
            return Err(ScimError::bad(
                "invalidFilter",
                format!("cannot filter users by {attribute}"),
            ));
        }
        users.retain(matches);
    }
    users.sort_by_key(|u| u.created_at);
    Ok(page(
        &query,
        users.iter().map(|u| user_resource(&base, u)).collect(),
    ))
}

async fn get_user(State(scim): State<Scim>, Path(id): Path<String>) -> Answer {
    let user = find_user(&scim.identity, &id)?;
    Ok(scim_json(
        StatusCode::OK,
        &user_resource(&scim.base(), &user),
    ))
}

async fn create_user(State(scim): State<Scim>, body: Bytes) -> Answer {
    let account = Account::from_resource(&parse_body(&body)?)?;
    let email = normalise_email(&account.email)?;
    if email == BREAK_GLASS_EMAIL {
        return Err(ScimError::bad("invalidValue", "this address is reserved"));
    }
    // Never an admin, and no password: they sign in through the provider.
    let made = scim
        .identity
        .create_user(&email, &account.name, None, false)?;
    let user = find_user(&scim.identity, &made.id.to_string())?;
    let user =
        apply_account(&scim.identity, &user, &Account { email, ..account })?;
    tracing::info!(
        event = "identity.scim_user_created",
        user = %user.id,
        "SCIM created an account"
    );
    let base = scim.base();
    Ok(created(
        &format!("{base}/Users/{}", user.id),
        &user_resource(&base, &user),
    ))
}

async fn replace_user(
    State(scim): State<Scim>,
    Path(id): Path<String>,
    body: Bytes,
) -> Answer {
    let user = find_user(&scim.identity, &id)?;
    let account = Account::from_resource(&parse_body(&body)?)?;
    let user = apply_account(&scim.identity, &user, &account)?;
    Ok(scim_json(
        StatusCode::OK,
        &user_resource(&scim.base(), &user),
    ))
}

/// The operations of a `PatchOp` body.
fn operations(
    body: &Value,
) -> Result<Vec<(String, Option<String>, Value)>, ScimError> {
    let list = body
        .get("Operations")
        .or_else(|| body.get("operations"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ScimError::bad("invalidSyntax", "a patch needs Operations")
        })?;
    list.iter()
        .map(|op| {
            let kind = op
                .get("op")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
                .filter(|k| matches!(k.as_str(), "add" | "replace" | "remove"))
                .ok_or_else(|| {
                    ScimError::bad(
                        "invalidSyntax",
                        "op must be add, replace or remove",
                    )
                })?;
            let path =
                op.get("path").and_then(Value::as_str).map(str::to_owned);
            Ok((kind, path, op.get("value").cloned().unwrap_or(Value::Null)))
        })
        .collect()
}

/// One attribute of a user patch.
fn patch_user_attribute(
    account: &mut Account,
    names: &mut (Option<String>, Option<String>),
    path: &str,
    value: &Value,
    remove: bool,
) -> Result<(), ScimError> {
    let lower = path.to_lowercase();
    match lower.as_str() {
        "active" => {
            account.active = if remove { true } else { boolean(value)? }
        }
        "externalid" => {
            account.external_id =
                if remove { None } else { external_id(value)? }
        }
        "displayname" | "name.formatted" => {
            if let Some(name) = text(value) {
                account.name = name;
            }
        }
        "name.givenname" => names.0 = text(value),
        "name.familyname" => names.1 = text(value),
        "name" => {
            if let Some(name) = value.get("formatted").and_then(text) {
                account.name = name;
            }
            if let Some(given) = value.get("givenName") {
                names.0 = text(given);
            }
            if let Some(family) = value.get("familyName") {
                names.1 = text(family);
            }
        }
        "username" => {
            if let Some(email) = text(value) {
                account.email = email;
            }
        }
        path if path.starts_with("emails") => {
            let email = value
                .as_array()
                .and_then(|l| l.first())
                .and_then(|e| e.get("value"))
                .or(Some(value))
                .and_then(text);
            if let Some(email) = email {
                account.email = email;
            }
        }
        // Attributes Vellum does not keep (title, phone numbers…).
        _ => {}
    }
    Ok(())
}

async fn patch_user(
    State(scim): State<Scim>,
    Path(id): Path<String>,
    body: Bytes,
) -> Answer {
    let user = find_user(&scim.identity, &id)?;
    let mut account = Account::of(&user);
    let mut names = (None, None);
    for (op, path, value) in operations(&parse_body(&body)?)? {
        let remove = op == "remove";
        match (path, &value) {
            (Some(path), _) => {
                patch_user_attribute(
                    &mut account,
                    &mut names,
                    &path,
                    &value,
                    remove,
                )?;
            }
            (None, Value::Object(map)) if !remove => {
                for (key, value) in map {
                    patch_user_attribute(
                        &mut account,
                        &mut names,
                        key,
                        value,
                        false,
                    )?;
                }
            }
            _ => {
                return Err(ScimError::bad(
                    "noTarget",
                    "a remove needs a path",
                ));
            }
        }
    }
    let parts: Vec<String> = <[Option<String>; 2]>::from(names)
        .into_iter()
        .flatten()
        .collect();
    if !parts.is_empty() {
        account.name = parts.join(" ");
    }
    let user = apply_account(&scim.identity, &user, &account)?;
    Ok(scim_json(
        StatusCode::OK,
        &user_resource(&scim.base(), &user),
    ))
}

async fn delete_user(
    State(scim): State<Scim>,
    Path(id): Path<String>,
) -> Answer {
    let user = find_user(&scim.identity, &id)?;
    let next = Account {
        active: false,
        external_id: None,
        ..Account::of(&user)
    };
    let _ = apply_account(&scim.identity, &user, &next)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// --- Groups ---

fn find_group(identity: &Identity, id: &str) -> Result<Group, ScimError> {
    let id: ObjectId = id.parse().map_err(|_| ScimError::not_found())?;
    identity
        .store()
        .read(|s| s.groups.get(&id.to_string()).cloned())
        .ok_or_else(ScimError::not_found)
}

async fn group_resource(
    scim: &Scim,
    group: &Group,
    with_members: bool,
) -> Result<Value, ScimError> {
    let base = scim.base();
    let mut resource = json!({
        "schemas": [GROUP_SCHEMA],
        "id": group.id.to_string(),
        "displayName": group.name,
        "meta": {
            "resourceType": "Group",
            "created": group.created_at.to_string(),
            "location": format!("{base}/Groups/{}", group.id),
        },
    });
    let Some(map) = resource.as_object_mut() else {
        return Ok(resource);
    };
    if let Some(id) = &group.external_id {
        let _ = map.insert("externalId".to_owned(), Value::from(id.clone()));
    }
    if with_members && let Some(port) = &scim.members {
        let ids = port.members(group.id).await?;
        let people = scim.identity.store().read(|s| {
            ids.iter()
                .filter_map(|id| s.users.get(id))
                .map(|u| {
                    json!({
                        "value": u.id.to_string(),
                        "display": u.name,
                        "$ref": format!("{base}/Users/{}", u.id)
                    })
                })
                .collect::<Vec<_>>()
        });
        let _ = map.insert("members".to_owned(), Value::from(people));
    }
    Ok(resource)
}

/// The people a `members` value names; each must be an active account.
fn member_ids(
    identity: &Identity,
    value: &Value,
) -> Result<BTreeSet<UserId>, ScimError> {
    let list = match value {
        Value::Array(list) => list.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    };
    list.iter()
        .map(|m| {
            let id =
                m.get("value").and_then(Value::as_str).ok_or_else(|| {
                    ScimError::bad("invalidValue", "each member needs a value")
                })?;
            let user = find_user(identity, id).map_err(|_| {
                ScimError::bad("invalidValue", format!("no account {id}"))
            })?;
            if user.disabled {
                return Err(ScimError::bad(
                    "invalidValue",
                    format!("{} is blocked", user.name),
                ));
            }
            Ok(user.id)
        })
        .collect()
}

/// `members[value eq "<id>"]`, the path providers use to remove one.
fn member_path(path: &str) -> Option<String> {
    let inner = path.strip_prefix("members[")?.strip_suffix(']')?;
    parse_filter(inner)
        .ok()
        .filter(|(attribute, _)| attribute == "value")
        .map(|(_, value)| value)
}

async fn set_members(
    scim: &Scim,
    group: ObjectId,
    add: &BTreeSet<UserId>,
    remove: &BTreeSet<UserId>,
) -> Result<(), ScimError> {
    if add.is_empty() && remove.is_empty() {
        return Ok(());
    }
    let add: Vec<UserId> = add.iter().copied().collect();
    let remove: Vec<UserId> = remove.iter().copied().collect();
    scim.port()?.change(group, &add, &remove).await?;
    Ok(())
}

async fn list_groups(
    State(scim): State<Scim>,
    Query(query): Query<ListQuery>,
) -> Answer {
    let filter = query.filter.as_deref().map(parse_filter).transpose()?;
    let mut groups: Vec<Group> = scim
        .identity
        .store()
        .read(|s| s.groups.values().cloned().collect());
    if let Some((attribute, value)) = &filter {
        if !matches!(attribute.as_str(), "displayname" | "externalid" | "id") {
            return Err(ScimError::bad(
                "invalidFilter",
                format!("cannot filter groups by {attribute}"),
            ));
        }
        groups.retain(|g| match attribute.as_str() {
            "displayname" => g.name.to_lowercase() == value.to_lowercase(),
            "externalid" => g.external_id.as_deref() == Some(value.as_str()),
            _ => g.id.to_string() == *value,
        });
    }
    groups.sort_by_key(|g| g.created_at);
    let with_members = !query
        .excluded_attributes
        .as_deref()
        .is_some_and(|e| e.to_lowercase().contains("members"));
    let mut resources = Vec::with_capacity(groups.len());
    for group in &groups {
        resources.push(group_resource(&scim, group, with_members).await?);
    }
    Ok(page(&query, resources))
}

async fn get_group(
    State(scim): State<Scim>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Answer {
    let group = find_group(&scim.identity, &id)?;
    let with_members = !query
        .excluded_attributes
        .as_deref()
        .is_some_and(|e| e.to_lowercase().contains("members"));
    Ok(scim_json(
        StatusCode::OK,
        &group_resource(&scim, &group, with_members).await?,
    ))
}

fn set_group_external_id(
    identity: &Identity,
    id: ObjectId,
    external: Option<String>,
) -> Result<(), ScimError> {
    identity.store().write(|s| {
        if let Some(group) = s.groups.get_mut(&id.to_string()) {
            group.external_id = external;
        }
    })?;
    Ok(())
}

async fn create_group(State(scim): State<Scim>, body: Bytes) -> Answer {
    let body = parse_body(&body)?;
    let name = body.get("displayName").and_then(text).ok_or_else(|| {
        ScimError::bad("invalidValue", "displayName is required")
    })?;
    let members = member_ids(
        &scim.identity,
        body.get("members").unwrap_or(&Value::Null),
    )?;
    if !members.is_empty() {
        let _ = scim.port()?;
    }
    let made = scim.identity.create_group(&name, "")?;
    let external = body
        .get("externalId")
        .map(external_id)
        .transpose()?
        .flatten();
    set_group_external_id(&scim.identity, made.id, external)?;
    set_members(&scim, made.id, &members, &BTreeSet::new()).await?;
    tracing::info!(
        event = "identity.scim_group_created",
        group = %made.id,
        "SCIM created a group"
    );
    let group = find_group(&scim.identity, &made.id.to_string())?;
    let base = scim.base();
    Ok(created(
        &format!("{base}/Groups/{}", group.id),
        &group_resource(&scim, &group, true).await?,
    ))
}

async fn current_members(
    scim: &Scim,
    group: ObjectId,
) -> Result<BTreeSet<UserId>, ScimError> {
    Ok(scim.port()?.members(group).await?)
}

async fn replace_group(
    State(scim): State<Scim>,
    Path(id): Path<String>,
    body: Bytes,
) -> Answer {
    let group = find_group(&scim.identity, &id)?;
    let body = parse_body(&body)?;
    let name = body.get("displayName").and_then(text).ok_or_else(|| {
        ScimError::bad("invalidValue", "displayName is required")
    })?;
    let wanted = member_ids(
        &scim.identity,
        body.get("members").unwrap_or(&Value::Null),
    )?;
    let _ = scim.identity.update_group(group.id, Some(&name), None)?;
    let external = body
        .get("externalId")
        .map(external_id)
        .transpose()?
        .flatten();
    set_group_external_id(&scim.identity, group.id, external)?;
    if body.get("members").is_some() {
        let now = current_members(&scim, group.id).await?;
        let add = wanted.difference(&now).copied().collect();
        let remove = now.difference(&wanted).copied().collect();
        set_members(&scim, group.id, &add, &remove).await?;
    }
    let group = find_group(&scim.identity, &id)?;
    Ok(scim_json(
        StatusCode::OK,
        &group_resource(&scim, &group, true).await?,
    ))
}

/// What a group patch changes.
#[derive(Default)]
struct GroupChange {
    name: Option<String>,
    /// Whether the external ID changes, and to what.
    set_external_id: bool,
    external_id: Option<String>,
    add: BTreeSet<UserId>,
    remove: BTreeSet<UserId>,
    replace: Option<BTreeSet<UserId>>,
    remove_all: bool,
}

fn patch_group_attribute(
    identity: &Identity,
    change: &mut GroupChange,
    op: &str,
    path: &str,
    value: &Value,
) -> Result<(), ScimError> {
    if let Some(id) = member_path(path) {
        let user: UserId = id
            .parse()
            .map_err(|_| ScimError::bad("invalidValue", "not an account ID"))?;
        let _ = change.remove.insert(user);
        return Ok(());
    }
    match (op, path.to_lowercase().as_str()) {
        ("remove", "members") if value.is_null() => change.remove_all = true,
        ("remove", "members") => change.remove.extend(member_ids_any(value)?),
        ("add", "members") => change.add.extend(member_ids(identity, value)?),
        ("replace", "members") => {
            change.replace = Some(member_ids(identity, value)?)
        }
        ("remove", "externalid") => {
            change.set_external_id = true;
            change.external_id = None;
        }
        (_, "externalid") => {
            change.set_external_id = true;
            change.external_id = external_id(value)?;
        }
        (_, "displayname") => change.name = text(value),
        _ => {}
    }
    Ok(())
}

/// Member IDs to remove: blocked or unknown accounts may be named too.
fn member_ids_any(value: &Value) -> Result<BTreeSet<UserId>, ScimError> {
    let list = match value {
        Value::Array(list) => list.clone(),
        other => vec![other.clone()],
    };
    list.iter()
        .map(|m| {
            m.get("value")
                .and_then(Value::as_str)
                .and_then(|id| id.parse().ok())
                .ok_or_else(|| {
                    ScimError::bad(
                        "invalidValue",
                        "each member needs an account ID",
                    )
                })
        })
        .collect()
}

async fn patch_group(
    State(scim): State<Scim>,
    Path(id): Path<String>,
    body: Bytes,
) -> Answer {
    let group = find_group(&scim.identity, &id)?;
    let mut change = GroupChange::default();
    for (op, path, value) in operations(&parse_body(&body)?)? {
        match (path, &value) {
            (Some(path), _) => {
                patch_group_attribute(
                    &scim.identity,
                    &mut change,
                    &op,
                    &path,
                    &value,
                )?;
            }
            (None, Value::Object(map)) if op != "remove" => {
                for (key, value) in map {
                    patch_group_attribute(
                        &scim.identity,
                        &mut change,
                        &op,
                        key,
                        value,
                    )?;
                }
            }
            _ => {
                return Err(ScimError::bad(
                    "noTarget",
                    "a remove needs a path",
                ));
            }
        }
    }
    if let Some(name) = &change.name {
        let _ = scim.identity.update_group(group.id, Some(name), None)?;
    }
    if change.set_external_id {
        set_group_external_id(
            &scim.identity,
            group.id,
            change.external_id.take(),
        )?;
    }
    let membership_changed = change.replace.is_some()
        || change.remove_all
        || !change.add.is_empty()
        || !change.remove.is_empty();
    if membership_changed {
        let now = current_members(&scim, group.id).await?;
        let mut wanted = if change.remove_all {
            BTreeSet::new()
        } else {
            change.replace.clone().unwrap_or_else(|| now.clone())
        };
        wanted.extend(change.add.iter().copied());
        for user in &change.remove {
            let _ = wanted.remove(user);
        }
        let add = wanted.difference(&now).copied().collect();
        let remove = now.difference(&wanted).copied().collect();
        set_members(&scim, group.id, &add, &remove).await?;
    }
    let group = find_group(&scim.identity, &id)?;
    Ok(scim_json(
        StatusCode::OK,
        &group_resource(&scim, &group, true).await?,
    ))
}

async fn delete_group(
    State(scim): State<Scim>,
    Path(id): Path<String>,
) -> Answer {
    let group = find_group(&scim.identity, &id)?;
    // Members go first, so access through the group ends even if the
    // record could not be removed; a retry finishes the job.
    if let Some(port) = &scim.members {
        let everyone: Vec<UserId> =
            port.members(group.id).await?.into_iter().collect();
        if !everyone.is_empty() {
            port.change(group.id, &[], &everyone).await?;
        }
    }
    scim.identity.forget_group(group.id)?;
    tracing::info!(
        event = "identity.scim_group_deleted",
        group = %group.id,
        "SCIM deleted a group"
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests fail loudly")]

    use super::*;

    #[test]
    fn filters_are_one_equality() {
        assert_eq!(
            parse_filter(r#"userName eq "priya@northstar.dev""#).unwrap(),
            ("username".to_owned(), "priya@northstar.dev".to_owned())
        );
        assert_eq!(
            parse_filter(r#"displayName EQ "Sales \"EMEA\"""#)
                .unwrap()
                .1,
            r#"Sales "EMEA""#
        );
        for bad in [
            r#"userName co "a""#,
            r#"userName eq "a" or userName eq "b""#,
            "userName eq a",
            r#"userName eq "a"b""#,
            "",
        ] {
            assert!(parse_filter(bad).is_err(), "{bad}");
        }
        assert_eq!(
            member_path(r#"members[value eq "u1"]"#).as_deref(),
            Some("u1")
        );
        assert_eq!(member_path("members"), None);
    }

    #[test]
    fn active_accepts_the_strings_providers_send() {
        assert!(!boolean(&json!("False")).unwrap());
        assert!(boolean(&json!(true)).unwrap());
        assert!(boolean(&json!(1)).is_err());
    }
}
