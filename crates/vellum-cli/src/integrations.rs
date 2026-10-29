// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The integration receivers (GitHub, GitLab, Slack, Sentry and inbound
//! mail) wired into `vellum serve`.
//!
//! Service crates cannot depend on each other, so this module adapts the
//! Work service to the integrations' ports:
//!
//! - [`ServiceWork`] runs each command inside [`WorkService::transact`] as
//!   the integration's bot user, so a pull request moving an issue is
//!   written, checked and announced exactly like a move made in the app.
//! - [`StoreLinks`] keeps what Git linked to each issue in
//!   `.vellum/integrations/links/<KEY>.json` in the data folder.
//!
//! Secrets come only from the environment ([`SECRET_VARS`]); everything
//! else comes from `.vellum/integrations.yaml`. A provider with no secret
//! stays off and its route answers `404`, and with no secret at all the
//! routes are not mounted.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt as _;
use serde::Deserialize;
use vellum_integrations::git::Mapping;
use vellum_integrations::git::automation::IssueLinks;
use vellum_integrations::git::event::{PrRef, Provider};
use vellum_integrations::git::service::{LinkStore, Repository, Settings};
use vellum_integrations::intake::Routing;
use vellum_integrations::intake::email::EmailSettings;
use vellum_integrations::model::{
    IssueDraft, IssueKey, IssueSnapshot, WorkCommands, WorkError, WorkflowState,
};
use vellum_integrations::receive::{ReceiverConfig, Receivers};
use vellum_integrations::work::steps;
use vellum_kernel::{Error, ErrorKind};
use vellum_store::{BlobPath, BlobStore, Precondition, StoreError};
use vellum_work::UserRef;
use vellum_work_service::WorkService;

/// The GitHub webhook secret.
pub const GITHUB_SECRET_VAR: &str = "VELLUM_GITHUB_WEBHOOK_SECRET";
/// The GitLab webhook token.
pub const GITLAB_TOKEN_VAR: &str = "VELLUM_GITLAB_WEBHOOK_TOKEN";
/// The Slack app's signing secret.
pub const SLACK_SECRET_VAR: &str = "VELLUM_SLACK_SIGNING_SECRET";
/// The Sentry integration's client secret.
pub const SENTRY_SECRET_VAR: &str = "VELLUM_SENTRY_CLIENT_SECRET";
/// The bearer token the inbound mail adapter sends.
pub const MAIL_TOKEN_VAR: &str = "VELLUM_MAIL_TOKEN";
/// The secret that signs reply addresses.
pub const REPLY_SECRET_VAR: &str = "VELLUM_MAIL_REPLY_SECRET";

/// Every variable that holds an integration secret.
pub const SECRET_VARS: [&str; 6] = [
    GITHUB_SECRET_VAR,
    GITLAB_TOKEN_VAR,
    SLACK_SECRET_VAR,
    SENTRY_SECRET_VAR,
    MAIL_TOKEN_VAR,
    REPLY_SECRET_VAR,
];

/// Where the non-secret settings live, inside the data folder.
pub const SETTINGS_PATH: &str = ".vellum/integrations.yaml";
/// The folder holding each issue's Git links.
const LINKS_DIR: &str = ".vellum/integrations/links";
/// The bot user integrations act as when the settings name none.
const DEFAULT_BOT: &str = "integrations";

/// The receivers' routes, to mount cross-site at `/hooks`, or `None` when
/// no integration secret is set.
///
/// # Errors
///
/// An unreadable or invalid settings file.
pub async fn hooks(
    store: &Arc<dyn BlobStore>,
    work: &Arc<WorkService>,
    var: impl Fn(&str) -> Option<String> + Send + Sync,
) -> Result<Option<axum::Router>, Error> {
    let secret = |name: &str| var(name).map(|v| v.trim().to_owned()).unwrap_or_default();
    if SECRET_VARS.iter().all(|v| secret(v).is_empty()) {
        return Ok(None);
    }
    let file = load_settings(store.as_ref()).await?;
    let bot = UserRef::parse(file.bot.as_deref().unwrap_or(DEFAULT_BOT)).map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("{SETTINGS_PATH}: bad bot handle: {e}"),
        )
    })?;
    let config = file.into_config(Secrets {
        github: secret(GITHUB_SECRET_VAR),
        gitlab: secret(GITLAB_TOKEN_VAR),
        slack: secret(SLACK_SECRET_VAR),
        sentry: secret(SENTRY_SECRET_VAR),
        mail: secret(MAIL_TOKEN_VAR),
        reply: secret(REPLY_SECRET_VAR),
    })?;
    let mut config = config;
    default_mappings(work, &mut config.git).await;
    tracing::info!(?config, "integration receivers mounted at /hooks");
    let receivers = Receivers::new(
        ServiceWork::new(Arc::clone(work), bot),
        StoreLinks::new(Arc::clone(store)),
        config,
    );
    Ok(Some(Arc::new(receivers).router()))
}

/// Gives every connected team without a mapping the default one for its
/// workflow (In Progress on a branch, In Review on a pull request, Done on
/// merge). Teams created after startup get theirs on the next start.
async fn default_mappings(work: &WorkService, git: &mut Settings) {
    let teams: BTreeSet<String> = git
        .repositories
        .iter()
        .flat_map(|r| r.teams.iter().cloned())
        .filter(|t| !git.mappings.contains_key(t))
        .collect();
    for team in teams {
        match work.read(|graph, _| steps::states(graph, &team)).await {
            Ok(states) => {
                let _ = git.mappings.insert(team, Mapping::default_for(&states));
            }
            Err(error) => {
                tracing::warn!(%team, %error, "connected team has no workflow; Git will not move its issues");
            }
        }
    }
}

async fn load_settings(store: &dyn BlobStore) -> Result<SettingsFile, Error> {
    let path = blob_path(SETTINGS_PATH)?;
    match store.get(&path).await {
        Ok(blob) => {
            let text = std::str::from_utf8(&blob.bytes).map_err(|_| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("{SETTINGS_PATH} is not UTF-8"),
                )
            })?;
            serde_saphyr::from_str(text)
                .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("{SETTINGS_PATH}: {e}")))
        }
        Err(StoreError::NotFound(_)) => {
            tracing::warn!(
                "integration secrets are set but {SETTINGS_PATH} is missing; \
                 no repository or intake team is connected"
            );
            Ok(SettingsFile::default())
        }
        Err(e) => Err(Error::new(
            ErrorKind::Unavailable,
            format!("cannot read {SETTINGS_PATH}: {e}"),
        )),
    }
}

/// The secrets, read from the environment.
struct Secrets {
    github: String,
    gitlab: String,
    slack: String,
    sentry: String,
    mail: String,
    reply: String,
}

/// `.vellum/integrations.yaml`: everything but the secrets.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsFile {
    /// The handle integrations act as.
    bot: Option<String>,
    /// Connected repositories.
    repositories: Vec<RepositoryEntry>,
    /// Per-team state mapping; connected teams left out get the default
    /// mapping for their workflow.
    mappings: BTreeMap<String, Mapping>,
    /// Where Slack, Sentry and mail intake lands.
    intake: IntakeEntry,
    /// Sentry project slug to team key.
    sentry_projects: BTreeMap<String, String>,
    /// Intake and reply addresses.
    email: EmailEntry,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryEntry {
    /// `github` or `gitlab`.
    provider: String,
    /// `owner/repo` or the GitLab project path.
    repo: String,
    /// Team keys the repository may act on.
    teams: BTreeSet<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct IntakeEntry {
    default_team: String,
    teams: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EmailEntry {
    intake_domain: String,
    plus_prefix: String,
    reply_domain: String,
}

impl Default for EmailEntry {
    fn default() -> Self {
        Self {
            intake_domain: String::new(),
            plus_prefix: "issues".to_owned(),
            reply_domain: String::new(),
        }
    }
}

impl SettingsFile {
    fn into_config(self, secrets: Secrets) -> Result<ReceiverConfig, Error> {
        let upper = |teams: BTreeSet<String>| -> BTreeSet<String> {
            teams.iter().map(|t| t.to_ascii_uppercase()).collect()
        };
        let mut repositories = Vec::with_capacity(self.repositories.len());
        for entry in self.repositories {
            let provider = match entry.provider.to_ascii_lowercase().as_str() {
                "github" => Provider::GitHub,
                "gitlab" => Provider::GitLab,
                other => {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        format!("{SETTINGS_PATH}: unknown provider {other:?}"),
                    ));
                }
            };
            repositories.push(Repository {
                provider,
                repo: entry.repo,
                teams: upper(entry.teams),
            });
        }
        let routing = Routing {
            default_team: self.intake.default_team.to_ascii_uppercase(),
            teams: upper(self.intake.teams),
        };
        Ok(ReceiverConfig {
            github_secret: secrets.github.into_bytes(),
            gitlab_token: secrets.gitlab,
            slack_signing_secret: secrets.slack.into_bytes(),
            sentry_client_secret: secrets.sentry.into_bytes(),
            mail_token: secrets.mail.into_bytes(),
            git: Settings {
                repositories,
                mappings: self
                    .mappings
                    .into_iter()
                    .map(|(team, m)| (team.to_ascii_uppercase(), m))
                    .collect(),
            },
            sentry_projects: self
                .sentry_projects
                .into_iter()
                .map(|(project, team)| (project, team.to_ascii_uppercase()))
                .collect(),
            email: EmailSettings {
                routing: routing.clone(),
                intake_domain: self.email.intake_domain,
                plus_prefix: self.email.plus_prefix,
                reply_domain: self.email.reply_domain,
                reply_secret: secrets.reply.into_bytes(),
            },
            routing,
        })
    }
}

/// [`WorkCommands`] over the running Work service.
pub struct ServiceWork {
    work: Arc<WorkService>,
    bot: UserRef,
}

impl std::fmt::Debug for ServiceWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceWork")
            .field("bot", &self.bot)
            .finish_non_exhaustive()
    }
}

impl ServiceWork {
    /// Commands acting as `bot` through `work`.
    #[must_use]
    pub const fn new(work: Arc<WorkService>, bot: UserRef) -> Self {
        Self { work, bot }
    }

    /// Runs one step in a transaction as `actor`. When the step fails,
    /// the service puts memory back and the step's own error is returned,
    /// so a missing issue stays "not found" for the receiver.
    async fn run<R: Send>(
        &self,
        actor: UserRef,
        step: impl FnOnce(
            &mut vellum_work::WorkGraph,
            &mut vellum_work::Comments,
            &vellum_work::Ctx,
        ) -> Result<vellum_work_service::Outcome<R>, WorkError>
        + Send,
    ) -> Result<R, WorkError> {
        let mut failed = None;
        let result = self
            .work
            .transact(actor, |graph, comments, ctx| {
                step(graph, comments, ctx).map_err(|e| {
                    let kernel = to_kernel(&e);
                    failed = Some(e);
                    kernel
                })
            })
            .await;
        result.map_err(|e| failed.unwrap_or_else(|| from_kernel(&e)))
    }
}

impl WorkCommands for ServiceWork {
    async fn issue(&self, key: &IssueKey) -> Result<Option<IssueSnapshot>, WorkError> {
        self.work.read(|graph, _| steps::snapshot(graph, key)).await
    }

    async fn states(&self, team: &str) -> Result<Vec<WorkflowState>, WorkError> {
        self.work.read(|graph, _| steps::states(graph, team)).await
    }

    async fn set_state(
        &self,
        key: &IssueKey,
        state_id: &str,
        reason: &str,
    ) -> Result<(), WorkError> {
        // The reason ("GitHub northstar/api#12 merged") goes to the audit
        // trail through this span.
        let span = tracing::info_span!("work.set_state", issue = %key, state = state_id, reason);
        tracing::Instrument::instrument(
            self.run(self.bot.clone(), |graph, _, ctx| {
                let changes = steps::set_state(graph, key, state_id, ctx)?;
                Ok((changes, None, ()))
            }),
            span,
        )
        .await
    }

    async fn create_issue(&self, draft: IssueDraft) -> Result<IssueKey, WorkError> {
        let span = tracing::info_span!("work.create_issue", team = %draft.team);
        tracing::Instrument::instrument(
            self.run(self.bot.clone(), |graph, _, ctx| {
                let (key, changes) = steps::create_issue(graph, draft, ctx)?;
                Ok((changes.unwrap_or_default(), None, key))
            }),
            span,
        )
        .await
    }

    async fn add_comment(
        &self,
        key: &IssueKey,
        author: Option<&str>,
        body: &str,
    ) -> Result<(), WorkError> {
        let actor = steps::actor(author, &self.bot)?;
        let span = tracing::info_span!("work.add_comment", issue = %key);
        tracing::Instrument::instrument(
            self.run(actor, |graph, comments, ctx| {
                let (added, changes) = steps::add_comment(graph, comments, key, body, ctx)?;
                Ok((changes, Some(added), ()))
            }),
            span,
        )
        .await
    }
}

fn to_kernel(e: &WorkError) -> Error {
    match e {
        WorkError::NotFound(what) => Error::new(ErrorKind::NotFound, what.clone()),
        WorkError::Refused(why) => Error::new(ErrorKind::InvalidInput, why.clone()),
        WorkError::Unavailable(why) => Error::new(ErrorKind::Unavailable, why.clone()),
    }
}

fn from_kernel(e: &Error) -> WorkError {
    match e.kind() {
        ErrorKind::NotFound => WorkError::NotFound(e.to_string()),
        ErrorKind::Unavailable | ErrorKind::Timeout | ErrorKind::Internal => {
            WorkError::Unavailable(e.to_string())
        }
        _ => WorkError::Refused(e.to_string()),
    }
}

/// Git links per issue, as JSON files in the data folder.
///
/// All links are read once and kept in memory, so finding the issues of a
/// pull request does not read every file; every save writes through.
pub struct StoreLinks {
    store: Arc<dyn BlobStore>,
    cache: tokio::sync::Mutex<Option<BTreeMap<IssueKey, IssueLinks>>>,
}

impl std::fmt::Debug for StoreLinks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreLinks").finish_non_exhaustive()
    }
}

impl StoreLinks {
    /// Links kept in `store`.
    #[must_use]
    pub fn new(store: Arc<dyn BlobStore>) -> Self {
        Self {
            store,
            cache: tokio::sync::Mutex::new(None),
        }
    }

    async fn read_all(&self) -> Result<BTreeMap<IssueKey, IssueLinks>, WorkError> {
        let dir = blob_path(LINKS_DIR).map_err(|e| WorkError::Unavailable(e.to_string()))?;
        let mut entries = self.store.list(Some(&dir));
        let mut all = BTreeMap::new();
        while let Some(entry) = entries.next().await {
            let entry = match entry {
                Ok(entry) => entry,
                Err(StoreError::NotFound(_)) => break,
                Err(e) => return Err(WorkError::Unavailable(e.to_string())),
            };
            let Some(name) = entry.path.as_str().rsplit('/').next() else {
                continue;
            };
            let Some(key) = name
                .strip_suffix(".json")
                .and_then(|k| k.parse::<IssueKey>().ok())
            else {
                continue;
            };
            let blob = self
                .store
                .get(&entry.path)
                .await
                .map_err(|e| WorkError::Unavailable(e.to_string()))?;
            match serde_json::from_slice::<IssueLinks>(&blob.bytes) {
                Ok(links) => {
                    let _ = all.insert(key, links);
                }
                Err(error) => {
                    tracing::warn!(path = entry.path.as_str(), %error, "unreadable issue links skipped");
                }
            }
        }
        Ok(all)
    }

    /// Runs `f` on the loaded links, reading them first if needed.
    async fn with_links<T>(
        &self,
        f: impl FnOnce(&mut BTreeMap<IssueKey, IssueLinks>) -> T,
    ) -> Result<T, WorkError> {
        let mut cache = self.cache.lock().await;
        if cache.is_none() {
            *cache = Some(self.read_all().await?);
        }
        let out = f(cache.get_or_insert_with(BTreeMap::new));
        drop(cache);
        Ok(out)
    }
}

impl LinkStore for StoreLinks {
    async fn load(&self, key: &IssueKey) -> Result<IssueLinks, WorkError> {
        self.with_links(|all| all.get(key).cloned().unwrap_or_default())
            .await
    }

    async fn save(&self, key: &IssueKey, links: &IssueLinks) -> Result<(), WorkError> {
        let path = blob_path(&format!("{LINKS_DIR}/{key}.json"))
            .map_err(|e| WorkError::Unavailable(e.to_string()))?;
        let bytes =
            serde_json::to_vec_pretty(links).map_err(|e| WorkError::Unavailable(e.to_string()))?;
        // Write first, so memory never claims a link the disk lacks.
        let _ = self
            .store
            .put(&path, Bytes::from(bytes), Precondition::None)
            .await
            .map_err(|e| WorkError::Unavailable(e.to_string()))?;
        self.with_links(|all| {
            let _ = all.insert(key.clone(), links.clone());
        })
        .await
    }

    async fn issues_for_pr(&self, pr: &PrRef) -> Result<BTreeSet<IssueKey>, WorkError> {
        self.with_links(|all| {
            all.iter()
                .filter(|(_, links)| links.prs.iter().any(|l| &l.pr == pr))
                .map(|(key, _)| key.clone())
                .collect()
        })
        .await
    }
}

fn blob_path(path: &str) -> Result<BlobPath, Error> {
    BlobPath::new(path).map_err(|e| Error::new(ErrorKind::Internal, format!("{path}: {e}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hmac::{KeyInit as _, Mac as _};
    use serde_json::{Value, json};
    use tower::ServiceExt as _;
    use vellum_integrations::model::Source;
    use vellum_store::LocalStore;
    use vellum_work::{Changes, Team, TeamKey};
    use vellum_work_service::{EventLog, StaticDirectory, SystemClock};

    use super::*;

    const GITHUB: &str = "github-test-secret";

    async fn workspace(dir: &std::path::Path) -> (Arc<dyn BlobStore>, Arc<WorkService>) {
        let store: Arc<dyn BlobStore> = Arc::new(LocalStore::open(dir).await.unwrap());
        let (work, _) = WorkService::open(
            Arc::clone(&store),
            Arc::new(StaticDirectory::new()),
            Arc::new(EventLog::new()),
            Arc::new(SystemClock),
        )
        .await
        .unwrap();
        let work = Arc::new(work);
        let team = Team::new(
            "01JC00000000000000000000E1".parse().unwrap(),
            TeamKey::parse("ENG").unwrap(),
            "Engineering",
        )
        .unwrap();
        work.transact(UserRef::parse("admin").unwrap(), |graph, _, _| {
            graph.insert_team(team)?;
            let mut changes = Changes::default();
            let _ = changes.teams.insert(TeamKey::parse("ENG")?);
            Ok((changes, None, ()))
        })
        .await
        .unwrap();
        (store, work)
    }

    async fn write(store: &Arc<dyn BlobStore>, path: &str, text: &str) {
        let _ = store
            .put(
                &blob_path(path).unwrap(),
                Bytes::from(text.to_owned()),
                Precondition::None,
            )
            .await
            .unwrap();
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + Send + Sync + use<> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn signature(body: &[u8]) -> String {
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(GITHUB.as_bytes()).unwrap();
        mac.update(body);
        let hex = mac
            .finalize()
            .into_bytes()
            .iter()
            .fold(String::new(), |mut out, b| {
                use std::fmt::Write as _;
                let _ = write!(out, "{b:02x}");
                out
            });
        format!("sha256={hex}")
    }

    async fn send(
        router: &axum::Router,
        path: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> (StatusCode, Value) {
        let mut request = Request::post(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn no_secret_mounts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (store, work) = workspace(dir.path()).await;
        assert!(hooks(&store, &work, vars(&[])).await.unwrap().is_none());
        // Blank values count as unset.
        let blank = vars(&[(GITHUB_SECRET_VAR, "  \n")]);
        assert!(hooks(&store, &work, blank).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn bad_settings_stop_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let (store, work) = workspace(dir.path()).await;
        let on = || vars(&[(GITHUB_SECRET_VAR, GITHUB)]);
        write(
            &store,
            SETTINGS_PATH,
            "repositories:\n  - provider: bitbucket\n    repo: a/b\n    teams: [ENG]\n",
        )
        .await;
        assert!(hooks(&store, &work, on()).await.is_err());
        write(&store, SETTINGS_PATH, "secret: in-the-file\n").await;
        assert!(hooks(&store, &work, on()).await.is_err());
        write(&store, SETTINGS_PATH, "bot: Not A Handle\n").await;
        assert!(hooks(&store, &work, on()).await.is_err());
    }

    #[tokio::test]
    async fn a_github_delivery_moves_the_issue_through_the_work_service() {
        let dir = tempfile::tempdir().unwrap();
        let (store, work) = workspace(dir.path()).await;
        write(
            &store,
            SETTINGS_PATH,
            "repositories:\n  - provider: github\n    repo: northstar/api\n    teams: [eng]\n",
        )
        .await;
        let router = hooks(&store, &work, vars(&[(GITHUB_SECRET_VAR, GITHUB)]))
            .await
            .unwrap()
            .unwrap();
        let commands = ServiceWork::new(Arc::clone(&work), UserRef::parse("integrations").unwrap());
        let issue = commands
            .create_issue(IssueDraft::new("ENG", "Fix login", Source::App))
            .await
            .unwrap();
        assert_eq!(issue.to_string(), "ENG-1");

        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": "northstar/api", "default_branch": "main"},
            "pull_request": {
                "number": 12, "title": "Fix login", "body": "Fixes ENG-1",
                "html_url": "https://github.com/northstar/api/pull/12",
                "draft": false, "merged": false, "head": {"ref": "fix-login"}
            }
        }))
        .unwrap();
        let (status, json) = send(
            &router,
            "/github",
            &[
                ("X-GitHub-Event", "pull_request"),
                ("X-Hub-Signature-256", &signature(&body)),
            ],
            body,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{json}");
        assert_eq!(json["applied"], 1);
        let moved = commands.issue(&issue).await.unwrap().unwrap();
        assert_eq!(moved.state.id, "in-review");

        // The link outlives the process: a fresh store reads it back.
        let links = StoreLinks::new(Arc::clone(&store));
        let pr = PrRef {
            provider: Provider::GitHub,
            repo: "northstar/api".to_owned(),
            number: 12,
        };
        assert_eq!(
            links.issues_for_pr(&pr).await.unwrap(),
            std::iter::once(issue).collect()
        );

        // Providers without a secret stay refused.
        let (status, _) = send(&router, "/gitlab", &[], b"{}".to_vec()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn intake_is_filed_once_and_errors_keep_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let (_, work) = workspace(dir.path()).await;
        let commands = ServiceWork::new(work, UserRef::parse("integrations").unwrap());
        let alert = || {
            IssueDraft::new(
                "ENG",
                "TypeError in render",
                Source::Sentry {
                    issue_id: "42".to_owned(),
                },
            )
        };
        let first = commands.create_issue(alert()).await.unwrap();
        let again = commands.create_issue(alert()).await.unwrap();
        assert_eq!(first, again);

        commands
            .add_comment(&first, Some("zoe"), "Still happening.")
            .await
            .unwrap();
        let missing: IssueKey = "ENG-99".parse().unwrap();
        assert!(matches!(
            commands.add_comment(&missing, None, "hi").await,
            Err(WorkError::NotFound(_))
        ));
        assert!(matches!(
            commands.set_state(&first, "Not A State!", "test").await,
            Err(WorkError::Refused(_))
        ));
        assert!(matches!(
            commands.states("NOPE").await,
            Err(WorkError::NotFound(_))
        ));
    }
}
