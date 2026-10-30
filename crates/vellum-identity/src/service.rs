// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The identity operations, registered with the gateway.
//!
//! Everything that is plain JSON in and out goes through the typed
//! operation seam (D88), so it appears in REST, OpenAPI and `GraphQL`.
//! Sign-in, sign-out and the `OAuth` endpoints set cookies or redirect,
//! so they are in [`crate::http`].

use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::{
    BoxFuture, CredentialKind, HttpMethod, Operation, Readiness, RequestContext, Scope, Service,
};
use vellum_kernel::{Error, ErrorKind, Timestamp, UserId};

use crate::identity::{Identity, UserView};
use crate::providers::SsoButton;
use crate::store::SessionMethod;
use crate::webauthn::Registration;

/// How recent a sign-in admin actions and token creation need.
pub const FRESH_AUTH: Duration = Duration::from_mins(15);

/// The identity service as the gateway sees it.
#[derive(Debug, Clone)]
pub struct IdentityService {
    identity: Arc<Identity>,
}

impl IdentityService {
    /// Wraps the identity core.
    #[must_use]
    pub const fn new(identity: Arc<Identity>) -> Self {
        Self { identity }
    }
}

/// No input.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

/// An ID in the path.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ById {
    /// The ID.
    pub id: String,
}

/// Who is calling and what the workspace needs.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    /// Whether the workspace still needs its first administrator.
    pub needs_setup: bool,
    /// The signed-in user, if any.
    pub user: Option<UserView>,
    /// How the caller authenticated.
    pub credential: String,
    /// The caller's scopes.
    pub scopes: Vec<Scope>,
    /// The first single sign-on button, when a provider is configured.
    pub sso: Option<SsoButton>,
    /// Every single sign-on button, in order.
    pub providers: Vec<SsoButton>,
}

/// The first administrator's details.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupInput {
    /// The sign-in address.
    pub email: String,
    /// The display name.
    pub name: String,
    /// The password, at least 12 characters.
    pub password: String,
}

/// Profile changes.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMe {
    /// The new display name.
    pub name: String,
}

/// A session as listed.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    /// The session ID.
    pub id: String,
    /// How it was established.
    pub method: SessionMethod,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it was last used, to within a few minutes.
    pub last_seen_at: Timestamp,
    /// When it ends.
    pub expires_at: Timestamp,
    /// The browser that holds it.
    pub user_agent: Option<String>,
}

/// An API token as listed (never the secret).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenView {
    /// The token ID.
    pub id: String,
    /// Its label.
    pub name: String,
    /// The granted scopes.
    pub scopes: Vec<Scope>,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it stops working, if ever.
    pub expires_at: Option<Timestamp>,
    /// When it was last used, to within a few minutes.
    pub last_used_at: Option<Timestamp>,
}

/// A new API token.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateToken {
    /// A label, such as "CI" or "Claude Desktop".
    pub name: String,
    /// The scopes to grant, no more than your own.
    pub scopes: Vec<Scope>,
    /// Days until it expires; never when absent.
    pub expires_in_days: Option<u32>,
}

/// A created token: the secret is shown only once.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedToken {
    /// The token, starting `vlm_`. Store it now; it is not shown again.
    pub token: String,
    /// The token's details.
    pub details: TokenView,
}

/// What a new personal API token may have.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenPolicy {
    /// The longest expiry in days; `None` allows tokens that never expire.
    pub max_days: Option<u32>,
    /// The scopes you may grant.
    pub scopes: Vec<Scope>,
}

/// Who owns a token, as administrators see it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenOwner {
    /// The user's ID.
    pub id: UserId,
    /// The display name.
    pub name: String,
    /// The sign-in address.
    pub email: String,
}

/// An API token as administrators list it (never the secret).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminTokenView {
    /// The token's details.
    pub token: TokenView,
    /// Its owner; absent if the account is gone.
    pub owner: Option<TokenOwner>,
}

/// The cap on new tokens' expiry.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SetTokenPolicy {
    /// The longest expiry in days, 1 to 3650; `null` lifts the cap.
    pub max_days: Option<u32>,
}

/// A code from the authenticator app being added.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TotpCode {
    /// The 6-digit code the app shows now.
    pub code: String,
}

/// A passkey the browser just created.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddPasskey {
    /// The ticket from the ceremony's options.
    pub ticket: String,
    /// A label, such as "Laptop" or "Security key".
    pub name: String,
    /// What `navigator.credentials.create()` returned, binary fields in
    /// base64url.
    pub credential: Registration,
}

/// A new set of recovery codes, shown once.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCodes {
    /// The codes; each works once.
    pub recovery_codes: Vec<String>,
}

/// A user, by ID in the path.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserById {
    /// The user's ID.
    pub id: UserId,
}

/// How many sessions ended.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SignedOut {
    /// Sessions ended.
    pub sessions: usize,
}

/// A connected `OAuth` app.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    /// The grant ID.
    pub id: String,
    /// The client ID.
    pub client_id: String,
    /// The client's name.
    pub client_name: String,
    /// The granted scopes.
    pub scopes: Vec<Scope>,
    /// The resources it may call.
    pub audience: Vec<String>,
    /// When it was approved.
    pub created_at: Timestamp,
}

/// A user an administrator creates.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    /// The sign-in address.
    pub email: String,
    /// The display name.
    pub name: String,
    /// An initial password; absent for SSO-only accounts.
    pub password: Option<String>,
    /// Whether the user administers the workspace.
    #[serde(default)]
    pub admin: bool,
}

/// Changes an administrator makes to a user.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateUser {
    /// The user's ID.
    pub id: UserId,
    /// A new display name.
    pub name: Option<String>,
    /// Grant or remove administrator rights.
    pub admin: Option<bool>,
    /// Block or allow sign-in. Blocking ends every session and token.
    pub disabled: Option<bool>,
    /// Let an administrator keep signing in with a password while single
    /// sign-on is required (break-glass).
    #[serde(default, rename = "breakGlass")]
    pub break_glass: Option<bool>,
}

/// An acknowledgement.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Done {
    /// Always `true`.
    pub ok: bool,
}

const fn done() -> Done {
    Done { ok: true }
}

fn require_session(ctx: &RequestContext) -> Result<UserId, Error> {
    let user = ctx.principal.require_user()?;
    match ctx.principal.kind() {
        CredentialKind::Session | CredentialKind::BreakGlass => Ok(user),
        _ => Err(Error::new(
            ErrorKind::PermissionDenied,
            "this needs a browser session, not a token",
        )),
    }
}

const fn credential_name(kind: CredentialKind) -> &'static str {
    match kind {
        CredentialKind::Anonymous => "anonymous",
        CredentialKind::Session => "session",
        CredentialKind::ApiToken => "api_token",
        CredentialKind::BreakGlass => "break_glass",
        CredentialKind::ShareLink => "share_link",
    }
}

fn token_view(t: crate::store::ApiToken) -> TokenView {
    TokenView {
        id: t.id,
        name: t.name,
        scopes: t.scopes,
        created_at: t.created_at,
        expires_at: t.expires_at,
        last_used_at: t.last_used_at,
    }
}

impl Service for IdentityService {
    fn name(&self) -> &'static str {
        "identity"
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one declaration per operation reads best as a list"
    )]
    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "`{id}` is a path template, not a format string"
    )]
    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let id = &self.identity;
        let status = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move {
                    let user = ctx.principal.user_id().and_then(|u| id.user(u).ok());
                    let buttons = id.sign_in_buttons();
                    Ok(AuthStatus {
                        needs_setup: id.needs_setup(),
                        user,
                        credential: credential_name(ctx.principal.kind()).to_owned(),
                        scopes: ctx.principal.scopes().iter().collect(),
                        sso: buttons.first().cloned(),
                        providers: buttons,
                    })
                }
            }
        };
        let setup = {
            let id = Arc::clone(id);
            move |_: RequestContext, input: SetupInput| {
                let id = Arc::clone(&id);
                async move { id.setup(&input.email, &input.name, &input.password) }
            }
        };
        let update_me = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: UpdateMe| {
                let id = Arc::clone(&id);
                async move {
                    let user = require_session(&ctx)?;
                    id.update_user(user, Some(&input.name), None, None)
                }
            }
        };
        let sessions = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move {
                    let user = ctx.principal.require_user()?;
                    Ok(id
                        .sessions(user)
                        .into_iter()
                        .map(|s| SessionView {
                            id: s.id,
                            method: s.method,
                            created_at: s.created_at,
                            last_seen_at: s.last_seen_at,
                            expires_at: s.expires_at,
                            user_agent: s.user_agent,
                        })
                        .collect::<Vec<_>>())
                }
            }
        };
        let revoke_session = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: ById| {
                let id = Arc::clone(&id);
                async move {
                    id.revoke_session(ctx.principal.require_user()?, &input.id)?;
                    Ok(done())
                }
            }
        };
        let tokens = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move {
                    let user = ctx.principal.require_user()?;
                    Ok(id
                        .api_tokens(user)
                        .into_iter()
                        .map(token_view)
                        .collect::<Vec<_>>())
                }
            }
        };
        let create_token = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: CreateToken| {
                let id = Arc::clone(&id);
                async move {
                    let user = require_session(&ctx)?;
                    let (token, record) = id.create_api_token(
                        user,
                        &input.name,
                        &input.scopes,
                        input.expires_in_days,
                    )?;
                    Ok(CreatedToken {
                        token,
                        details: token_view(record),
                    })
                }
            }
        };
        let revoke_token = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: ById| {
                let id = Arc::clone(&id);
                async move {
                    id.revoke_api_token(ctx.principal.require_user()?, &input.id)?;
                    Ok(done())
                }
            }
        };
        let token_policy = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move {
                    let user = ctx.principal.require_user()?;
                    Ok(TokenPolicy {
                        max_days: id.max_token_days(),
                        scopes: id.token_scopes(user)?,
                    })
                }
            }
        };
        let admin_tokens = {
            let id = Arc::clone(id);
            move |_: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move {
                    Ok(id
                        .all_api_tokens()
                        .into_iter()
                        .map(|(token, owner)| AdminTokenView {
                            token: token_view(token),
                            owner: owner.map(|u| TokenOwner {
                                id: u.id,
                                name: u.name,
                                email: u.email,
                            }),
                        })
                        .collect::<Vec<_>>())
                }
            }
        };
        let admin_revoke_token = {
            let id = Arc::clone(id);
            move |_: RequestContext, input: ById| {
                let id = Arc::clone(&id);
                async move {
                    let _ = id.admin_revoke_api_token(&input.id)?;
                    Ok(done())
                }
            }
        };
        let admin_token_policy = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: SetTokenPolicy| {
                let id = Arc::clone(&id);
                async move {
                    let user = ctx.principal.require_user()?;
                    Ok(TokenPolicy {
                        max_days: id.set_max_token_days(input.max_days)?,
                        scopes: id.token_scopes(user)?,
                    })
                }
            }
        };
        let factors = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move { id.factors(require_session(&ctx)?) }
            }
        };
        let start_totp = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move { id.start_totp(require_session(&ctx)?) }
            }
        };
        let confirm_totp = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: TotpCode| {
                let id = Arc::clone(&id);
                async move { id.confirm_totp(require_session(&ctx)?, &input.code) }
            }
        };
        let remove_totp = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move { id.remove_totp(require_session(&ctx)?) }
            }
        };
        let start_passkey = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move { id.start_passkey(require_session(&ctx)?) }
            }
        };
        let add_passkey = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: AddPasskey| {
                let id = Arc::clone(&id);
                async move {
                    id.finish_passkey(
                        require_session(&ctx)?,
                        &input.ticket,
                        &input.name,
                        &input.credential,
                    )
                }
            }
        };
        let remove_passkey = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: ById| {
                let id = Arc::clone(&id);
                async move { id.remove_passkey(require_session(&ctx)?, &input.id) }
            }
        };
        let recovery_codes = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move {
                    Ok(RecoveryCodes {
                        recovery_codes: id.new_recovery_codes(require_session(&ctx)?)?,
                    })
                }
            }
        };
        let admin_reset_factors = {
            let id = Arc::clone(id);
            move |_: RequestContext, input: UserById| {
                let id = Arc::clone(&id);
                async move { id.reset_factors(input.id) }
            }
        };
        let admin_sign_out = {
            let id = Arc::clone(id);
            move |_: RequestContext, input: UserById| {
                let id = Arc::clone(&id);
                async move {
                    Ok(SignedOut {
                        sessions: id.sign_out_everywhere(input.id)?,
                    })
                }
            }
        };
        let apps = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move {
                    let user = ctx.principal.require_user()?;
                    Ok(id
                        .grants(user)
                        .into_iter()
                        .map(|g| AppView {
                            id: g.id,
                            client_id: g.client_id,
                            client_name: g.client_name,
                            scopes: g.scopes,
                            audience: g.audience,
                            created_at: g.created_at,
                        })
                        .collect::<Vec<_>>())
                }
            }
        };
        let revoke_app = {
            let id = Arc::clone(id);
            move |ctx: RequestContext, input: ById| {
                let id = Arc::clone(&id);
                async move {
                    id.revoke_grant(ctx.principal.require_user()?, &input.id)?;
                    Ok(done())
                }
            }
        };
        let users = {
            let id = Arc::clone(id);
            move |_: RequestContext, _: Empty| {
                let id = Arc::clone(&id);
                async move { Ok(id.users()) }
            }
        };
        let create_user = {
            let id = Arc::clone(id);
            move |_: RequestContext, input: CreateUser| {
                let id = Arc::clone(&id);
                async move {
                    id.create_user(
                        &input.email,
                        &input.name,
                        input.password.as_deref(),
                        input.admin,
                    )
                }
            }
        };
        let update_user = {
            let id = Arc::clone(id);
            move |_: RequestContext, input: UpdateUser| {
                let id = Arc::clone(&id);
                async move {
                    let user = id.update_user(
                        input.id,
                        input.name.as_deref(),
                        input.admin,
                        input.disabled,
                    )?;
                    match input.break_glass {
                        Some(on) => id.set_break_glass(input.id, on),
                        None => Ok(user),
                    }
                }
            }
        };
        vec![
            Operation::query("identity.status", "/v1/auth/status", Scope::Read, status)
                .allow_anonymous()
                .summary("Who is signed in, and whether the workspace needs set-up"),
            Operation::mutation(
                "identity.setup",
                HttpMethod::Post,
                "/v1/setup",
                Scope::Admin,
                setup,
            )
            .allow_anonymous()
            .summary("Create the first administrator of a new workspace"),
            Operation::mutation(
                "identity.update_me",
                HttpMethod::Patch,
                "/v1/me",
                Scope::Read,
                update_me,
            )
            .summary("Change your display name"),
            Operation::query(
                "identity.sessions",
                "/v1/me/sessions",
                Scope::Read,
                sessions,
            )
            .summary("List your signed-in browsers"),
            Operation::mutation(
                "identity.revoke_session",
                HttpMethod::Delete,
                "/v1/me/sessions/{id}",
                Scope::Read,
                revoke_session,
            )
            .summary("Sign out one of your browsers"),
            Operation::query("identity.tokens", "/v1/me/tokens", Scope::Read, tokens)
                .summary("List your API tokens"),
            Operation::mutation(
                "identity.create_token",
                HttpMethod::Post,
                "/v1/me/tokens",
                Scope::Read,
                create_token,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Create an API token; the secret is returned once"),
            Operation::mutation(
                "identity.revoke_token",
                HttpMethod::Delete,
                "/v1/me/tokens/{id}",
                Scope::Read,
                revoke_token,
            )
            .summary("Revoke one of your API tokens"),
            Operation::query(
                "identity.token_policy",
                "/v1/me/token-policy",
                Scope::Read,
                token_policy,
            )
            .summary("The scopes and longest expiry a new API token may have"),
            Operation::query(
                "identity.admin_tokens",
                "/v1/admin/tokens",
                Scope::Admin,
                admin_tokens,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("List everyone's API tokens"),
            Operation::mutation(
                "identity.admin_revoke_token",
                HttpMethod::Delete,
                "/v1/admin/tokens/{id}",
                Scope::Admin,
                admin_revoke_token,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Revoke anyone's API token"),
            Operation::mutation(
                "identity.admin_token_policy",
                HttpMethod::Put,
                "/v1/admin/token-policy",
                Scope::Admin,
                admin_token_policy,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Cap how long new API tokens may last"),
            Operation::query(
                "identity.factors",
                "/v1/me/second-factors",
                Scope::Read,
                factors,
            )
            .summary("Your authenticator app, passkeys and recovery codes"),
            Operation::mutation(
                "identity.start_totp",
                HttpMethod::Post,
                "/v1/me/second-factors/totp",
                Scope::Read,
                start_totp,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Start adding an authenticator app; confirm it with a code"),
            Operation::mutation(
                "identity.confirm_totp",
                HttpMethod::Post,
                "/v1/me/second-factors/totp/confirm",
                Scope::Read,
                confirm_totp,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Confirm the authenticator app with its first code"),
            Operation::mutation(
                "identity.remove_totp",
                HttpMethod::Delete,
                "/v1/me/second-factors/totp",
                Scope::Read,
                remove_totp,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Remove your authenticator app"),
            Operation::mutation(
                "identity.start_passkey",
                HttpMethod::Post,
                "/v1/me/second-factors/passkeys/options",
                Scope::Read,
                start_passkey,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Start adding a passkey: the options for the browser"),
            Operation::mutation(
                "identity.add_passkey",
                HttpMethod::Post,
                "/v1/me/second-factors/passkeys",
                Scope::Read,
                add_passkey,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Add the passkey the browser created"),
            Operation::mutation(
                "identity.remove_passkey",
                HttpMethod::Delete,
                "/v1/me/second-factors/passkeys/{id}",
                Scope::Read,
                remove_passkey,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Remove a passkey"),
            Operation::mutation(
                "identity.recovery_codes",
                HttpMethod::Post,
                "/v1/me/second-factors/recovery-codes",
                Scope::Read,
                recovery_codes,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Replace your recovery codes; the new ones are shown once"),
            Operation::mutation(
                "identity.admin_reset_factors",
                HttpMethod::Post,
                "/v1/admin/users/{id}/second-factors/reset",
                Scope::Admin,
                admin_reset_factors,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Remove someone's second factors and end their sessions"),
            Operation::mutation(
                "identity.admin_sign_out",
                HttpMethod::Post,
                "/v1/admin/users/{id}/sign-out",
                Scope::Admin,
                admin_sign_out,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("End every session of one person"),
            Operation::query("identity.apps", "/v1/me/apps", Scope::Read, apps)
                .summary("List apps you connected with OAuth"),
            Operation::mutation(
                "identity.revoke_app",
                HttpMethod::Delete,
                "/v1/me/apps/{id}",
                Scope::Read,
                revoke_app,
            )
            .summary("Disconnect an app"),
            Operation::query("identity.users", "/v1/admin/users", Scope::Admin, users)
                .fresh_auth(FRESH_AUTH)
                .summary("List every user"),
            Operation::mutation(
                "identity.create_user",
                HttpMethod::Post,
                "/v1/admin/users",
                Scope::Admin,
                create_user,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Create a user"),
            Operation::mutation(
                "identity.update_user",
                HttpMethod::Patch,
                "/v1/admin/users/{id}",
                Scope::Admin,
                update_user,
            )
            .fresh_auth(FRESH_AUTH)
            .summary("Rename, promote, demote, block or unblock a user"),
        ]
        .into_iter()
        .chain(crate::providers::operations(id))
        .collect()
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(async { Readiness::Ready })
    }
}
