// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Secret scanning on save in `vellum serve` (R-SE-6, D296).
//!
//! - [`SecretGuard`] is the drive's content check, so it sees every page
//!   the drive creates or saves: the REST API, live editing, models
//!   writing through MCP, and imports. It also checks text files attached
//!   to pages and every collection row written (D301). Under the admin's policy
//!   (Admin → Settings → Security) a finding is audited and the save goes
//!   through (`warn`, the default), or the save is refused with a typed
//!   `422` (`block`). `off` scans nothing.
//! - [`SecretsService`] serves `security.scan_secrets`, which the editor
//!   calls to show its warning banner with the kind and line of each
//!   finding.
//!
//! Neither ever logs, audits or returns the secret: only its kind, its
//! line and column, and a redacted preview for the writer's own banner.

use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_admin::settings::SecuritySettings;
use vellum_admin::{
    AdminService, AuditActor, AuditLog, NewAuditEvent, Outcome, SecretScanning,
};
use vellum_collections::service::RowCheck;
use vellum_contracts::{
    HttpMethod, Operation, Principal, RequestContext, Scope, Service,
};
use vellum_drive::{ContentCheck, DriveError, DriveResult};
use vellum_kernel::{Actor, Error, ErrorKind, SpaceId};
use vellum_security::secrets::{self, Finding, SECRETS_FOUND};
use vellum_store::BlobPath;

/// Most findings named in a refusal or audited per save.
const MAX_REPORTED: usize = 20;

/// The admin console's keyring type, as `vellum serve` builds it.
pub type Admin = AdminService<vellum_crypto::LocalKeyring>;

/// The policy and the audit log, attached once the admin console opens.
#[derive(Default)]
pub struct SecretGuard {
    admin: OnceLock<Arc<Admin>>,
    audit: OnceLock<Arc<dyn AuditLog>>,
    last: Mutex<Option<SecuritySettings>>,
}

impl std::fmt::Debug for SecretGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretGuard")
            .field("admin", &self.admin.get().is_some())
            .finish_non_exhaustive()
    }
}

/// The findings a policy keeps: those its allow-list does not excuse.
fn kept(text: &str, settings: &SecuritySettings) -> Vec<Finding> {
    secrets::scan(text)
        .into_iter()
        .filter(|f| {
            !settings.secret_allow.iter().any(|allow| {
                let allow = allow.trim();
                allow == f.rule.id()
                    || text
                        .get(f.range.clone())
                        .is_some_and(|secret| secret.contains(allow))
            })
        })
        .collect()
}

/// `github-token on line 3, jwt on line 9`.
fn summary(findings: &[Finding]) -> String {
    let mut parts: Vec<String> = findings
        .iter()
        .take(MAX_REPORTED)
        .map(|f| format!("{} on line {}", f.rule.id(), f.line))
        .collect();
    if findings.len() > MAX_REPORTED {
        parts.push(format!("and {} more", findings.len() - MAX_REPORTED));
    }
    parts.join(", ")
}

fn audit_actor(actor: &Actor) -> AuditActor {
    let (kind, user) = match actor {
        Actor::User { user } => ("user", Some(user.to_string())),
        Actor::Model { on_behalf_of } => {
            ("model", Some(on_behalf_of.to_string()))
        }
        Actor::External => ("external", None),
        _ => ("system", None),
    };
    AuditActor {
        kind: kind.to_owned(),
        user_id: user,
        via: None,
    }
}

impl SecretGuard {
    /// Starts following the admin console's settings and audit log.
    pub fn attach(&self, admin: Arc<Admin>) {
        let _ = self.audit.set(admin.audit_log());
        let _ = self.admin.set(admin);
    }

    /// The security settings now: the defaults until the admin console is
    /// open, and the last good read if it cannot be read. Read on every
    /// write, so a policy change applies to the next save.
    pub async fn settings(&self) -> SecuritySettings {
        let Some(admin) = self.admin.get() else {
            return SecuritySettings::default();
        };
        match admin.settings().await {
            Ok(versioned) => {
                let settings = versioned.settings.security;
                if let Ok(mut last) = self.last.lock() {
                    *last = Some(settings.clone());
                }
                settings
            }
            Err(error) => {
                tracing::warn!(%error, "secret scanning: admin settings \
                    unreadable, using the last ones");
                self.last
                    .lock()
                    .ok()
                    .and_then(|l| l.clone())
                    .unwrap_or_default()
            }
        }
    }

    async fn audit(
        &self,
        actor: &Actor,
        path: &BlobPath,
        findings: &[Finding],
        blocked: bool,
    ) {
        let outcome = if blocked {
            Outcome::Denied
        } else {
            Outcome::Success
        };
        for f in findings.iter().take(MAX_REPORTED) {
            let event =
                NewAuditEvent::new(audit_actor(actor), "security.secret_found")
                    .target(path.as_str())
                    .outcome(outcome)
                    .detail("rule", f.rule.id())
                    .detail("line", f.line)
                    .detail("column", f.column)
                    .detail("blocked", blocked);
            if let Some(log) = self.audit.get() {
                if let Err(error) = log.append(event).await {
                    tracing::warn!(%error, "secret finding not audited");
                }
            } else {
                tracing::info!(
                    target: "vellum::audit",
                    rule = f.rule.id(),
                    line = f.line,
                    path = path.as_str(),
                    blocked,
                    "secret found"
                );
            }
        }
    }
}

impl SecretGuard {
    /// Scans `text` under the policy now and audits what it finds. Under
    /// `block`, the refusal's message: it names kinds and lines only.
    async fn inspect(
        &self,
        actor: &Actor,
        path: &BlobPath,
        text: &str,
    ) -> Result<(), String> {
        let settings = self.settings().await;
        if settings.secret_scanning == SecretScanning::Off {
            return Ok(());
        }
        let findings = kept(text, &settings);
        if findings.is_empty() {
            return Ok(());
        }
        let blocked = settings.secret_scanning == SecretScanning::Block;
        self.audit(actor, path, &findings, blocked).await;
        if blocked {
            return Err(format!(
                "{SECRETS_FOUND}: {}. Remove them, or ask an admin to allow \
                    them.",
                summary(&findings)
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ContentCheck for SecretGuard {
    async fn check(
        &self,
        actor: &Actor,
        _space: SpaceId,
        path: &BlobPath,
        text: &str,
    ) -> DriveResult<()> {
        self.inspect(actor, path, text)
            .await
            .map_err(DriveError::Refused)
    }
}

/// Collection rows (D301): the same policy as pages, and the same `422`.
#[async_trait]
impl RowCheck for SecretGuard {
    async fn check(
        &self,
        principal: &Principal,
        path: &BlobPath,
        text: &str,
    ) -> Result<(), Error> {
        let actor = principal
            .user_id()
            .map_or(Actor::External, |user| Actor::User { user });
        self.inspect(&actor, path, text)
            .await
            .map_err(|message| Error::new(ErrorKind::InvalidInput, message))
    }
}

/// Text to scan.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SecretScanInput {
    /// The page's Markdown.
    pub text: String,
}

/// One likely secret, as the editor shows it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SecretFinding {
    /// What it looks like, such as `github-token`.
    pub rule: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
    /// Its length in characters, so the editor can select it to remove.
    pub length: usize,
    /// The first four characters and the length.
    pub redacted: String,
}

/// What a save of the text would meet.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SecretScanResult {
    /// `off`, `warn` or `block`.
    pub policy: SecretScanning,
    /// Likely secrets the allow-list does not excuse; empty when off.
    pub findings: Vec<SecretFinding>,
}

/// `security.scan_secrets`, for the editor's warning banner.
#[derive(Debug)]
pub struct SecretsService {
    guard: Arc<SecretGuard>,
}

impl SecretsService {
    /// Scans with `guard`'s policy.
    #[must_use]
    pub const fn new(guard: Arc<SecretGuard>) -> Self {
        Self { guard }
    }

    async fn scan(
        &self,
        input: SecretScanInput,
    ) -> Result<SecretScanResult, Error> {
        let settings = self.guard.settings().await;
        let findings = if settings.secret_scanning == SecretScanning::Off {
            Vec::new()
        } else {
            kept(&input.text, &settings)
                .into_iter()
                .map(|f| SecretFinding {
                    rule: f.rule.id().to_owned(),
                    line: f.line,
                    column: f.column,
                    length: input
                        .text
                        .get(f.range.clone())
                        .map_or(0, |s| s.chars().count()),
                    redacted: f.redacted,
                })
                .collect()
        };
        Ok(SecretScanResult {
            policy: settings.secret_scanning,
            findings,
        })
    }
}

impl Service for SecretsService {
    fn name(&self) -> &'static str {
        "security"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(&self);
        vec![
            Operation::mutation(
                "security.scan_secrets",
                HttpMethod::Post,
                "/v1/security/secrets/scan",
                Scope::Read,
                move |_ctx: RequestContext, input: SecretScanInput| {
                    let s = Arc::clone(&s);
                    async move { s.scan(input).await }
                },
            )
            .summary("Finds likely secrets in page text")
            .description(
                "Returns the workspace's secret-scanning policy and the likely \
                 secrets in the text (kind, line, column and a redacted \
                 preview), so the editor can warn before a save. Nothing is \
                 stored or audited.",
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GitHub token shaped string, built so this file holds none.
    fn token() -> String {
        format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8")
    }

    fn page(token: &str) -> String {
        format!("---\ntitle: Deploys\n---\n\nThe bot uses {token} to push.\n")
    }

    #[test]
    fn the_allow_list_skips_a_kind_or_a_known_value() {
        let text = page(&token());
        let mut settings = SecuritySettings::default();
        assert_eq!(kept(&text, &settings).len(), 1);
        settings.secret_allow = vec!["github-token".into()];
        assert!(kept(&text, &settings).is_empty());
        settings.secret_allow = vec!["a1B2c3D4".into()];
        assert!(kept(&text, &settings).is_empty());
        settings.secret_allow = vec!["jwt".into()];
        assert_eq!(kept(&text, &settings).len(), 1);
    }

    #[test]
    fn a_summary_names_kinds_and_lines_never_the_secret() {
        let text = page(&token());
        let s = summary(&kept(&text, &SecuritySettings::default()));
        assert_eq!(s, "github-token on line 5");
        assert!(!s.contains("a1B2"));
    }

    #[tokio::test]
    async fn without_an_admin_console_findings_warn_and_saves_go_through() {
        let guard = SecretGuard::default();
        let path = BlobPath::new("drives/shared/eng/deploys.md").unwrap();
        let text = page(&token());
        ContentCheck::check(
            &guard,
            &Actor::System,
            SpaceId::from_u128(1),
            &path,
            &text,
        )
        .await
        .unwrap();
        let service = SecretsService::new(Arc::new(guard));
        let result = service.scan(SecretScanInput { text }).await.unwrap();
        assert_eq!(result.policy, SecretScanning::Warn);
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].rule, "github-token");
        assert_eq!(result.findings[0].line, 5);
        assert!(!result.findings[0].redacted.contains("a1B2c3D4"));
    }

    #[tokio::test]
    async fn rows_meet_the_same_policy_as_pages() {
        let guard = SecretGuard::default();
        let path =
            BlobPath::new("drives/shared/eng/Bots.collection/rows/deploy.md")
                .unwrap();
        let principal = vellum_contracts::Principal::user(
            vellum_kernel::UserId::from_u128(7),
            vellum_contracts::CredentialKind::Session,
            vellum_contracts::ScopeSet::from_scopes([Scope::WriteDirect]),
            vellum_kernel::Timestamp::now(),
        );
        // Warn (no admin console yet): the row is written.
        RowCheck::check(&guard, &principal, &path, &page(&token()))
            .await
            .unwrap();
        RowCheck::check(&guard, &principal, &path, "no secrets here")
            .await
            .unwrap();
    }
}
