// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! My Work on the drive, identity and Work.
//!
//! [`DriveTasks`] is the server's side of `vellum_tasks::port::Workspace`:
//!
//! - **Lists** are pages in a `Tasks` folder at the top of their owner's
//!   My Drive. Sharing a list grants the other person editor access to
//!   that one page through the permission engine, so it shows up for
//!   them as well.
//! - **Checklists in documents** come from the pages the person can open,
//!   most recently changed first, that mention their `@handle`.
//! - **Assigned work** is the person's open issues and the open comment
//!   threads assigned to them on documents.
//! - **Secrets** (the feed token and the capture address) are stored as
//!   hashes in `<data>/.vellum/tasks/secrets.json`.
//! - **Mentions** are open comment threads where someone else named the
//!   person and they have not written since, on issues and on documents
//!   they can open, less the ones they dismissed.
//! - **Settings** (time zone, start of day, digest time) and dismissed
//!   mentions are kept in `<data>/.vellum/tasks/people.json`.
//! - **Slack users** are linked to Vellum handles in
//!   `<data>/.vellum/tasks/slack-users.json`, as `{"T…/U…": "handle"}`.
//!
//! Every read and write is checked against the caller's own access.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use vellum_authz::{
    Actor as AuthzActor, Authz, AuthzStore, Change, Id, Object, Relation, Subject, Tuple,
    TupleRelation,
};
use vellum_contracts::BoxFuture;
use vellum_drive::{Drive, DriveIndex, Filter, Node, NodeKind, Saved, Space, SpaceKind};
use vellum_identity::Identity;
use vellum_kernel::{
    Actor, CommentId, DocId, Error, ErrorKind, ObjectId, ShareTokenHash, SpaceId,
    Timestamp as KernelTime, UserId,
};
use vellum_store::{Base, BlobPath, BlobStore, Etag};
use vellum_tasks::model::{Due, IssueCategory, Origin, Priority, Source, Task, TaskFields};
use vellum_tasks::port::{Document, Profile, SecretKind, Workspace};
use vellum_tasks::projection::FileContext;
use vellum_tasks::settings::Preferences;

use crate::task_prefs::PeoplePrefs;
use vellum_work::workflow::Category;
use vellum_work::{Target, UserRef};
use vellum_work_service::WorkService;

/// The folder at the top of a My Drive that holds a person's lists.
pub const TASKS_FOLDER: &str = "Tasks";

/// The title of the list quick-add files into.
const INBOX: &str = "Inbox";

/// How many pages, newest first, are searched for checklists that
/// mention someone. Older documents drop out of My Work until edited.
pub const DOC_SCAN: usize = 200;

/// The most pages one My Work request looks at.
const FIND_LIMIT: usize = 5_000;

fn unavailable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what}: {e}"))
}

/// A stable 64-bit version for a store etag (FNV-1a), so the version a
/// calendar client sees survives a restart.
fn version_of(etag: &Etag) -> u64 {
    etag.as_str()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn to_jiff(time: KernelTime) -> jiff::Timestamp {
    jiff::Timestamp::from_millisecond(time.unix_ms()).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
}

/// The hashes and links My Work keeps outside the files.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SecretsFile {
    #[serde(default)]
    feed: BTreeMap<UserId, String>,
    #[serde(default)]
    capture: BTreeMap<UserId, String>,
}

impl SecretsFile {
    const fn map(&mut self, kind: SecretKind) -> &mut BTreeMap<UserId, String> {
        match kind {
            SecretKind::Feed => &mut self.feed,
            SecretKind::Capture => &mut self.capture,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let mut out = [0_u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The secret hashes, kept in a file (or in memory when there is none).
#[derive(Debug)]
struct Secrets {
    path: Option<PathBuf>,
    state: Mutex<SecretsFile>,
}

impl Secrets {
    fn open(dir: Option<&Path>) -> Result<Self, Error> {
        let path = dir.map(|d| d.join("secrets.json"));
        let state = match &path {
            Some(p) if p.exists() => {
                let bytes =
                    std::fs::read(p).map_err(|e| unavailable("cannot read task secrets", e))?;
                serde_json::from_slice(&bytes)
                    .map_err(|e| unavailable("task secrets are not valid JSON", e))?
            }
            _ => SecretsFile::default(),
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SecretsFile>, Error> {
        self.state
            .lock()
            .map_err(|_| Error::new(ErrorKind::Internal, "task secrets lock poisoned"))
    }

    fn set(
        &self,
        user: UserId,
        kind: SecretKind,
        hash: Option<ShareTokenHash>,
    ) -> Result<(), Error> {
        let bytes = {
            let mut state = self.lock()?;
            match hash {
                Some(h) => {
                    let _ = state.map(kind).insert(user, hex(h.as_bytes()));
                }
                None => {
                    let _ = state.map(kind).remove(&user);
                }
            }
            serde_json::to_vec_pretty(&*state)
                .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?
        };
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| unavailable("cannot save task secrets", e))?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|e| unavailable("cannot save task secrets", e))?;
        std::fs::rename(&temp, path).map_err(|e| unavailable("cannot save task secrets", e))
    }

    /// The owner of `hash`. Every stored hash is compared, in constant
    /// time, so the answer takes as long whether or not it matches.
    fn user_for(&self, kind: SecretKind, hash: ShareTokenHash) -> Result<Option<UserId>, Error> {
        let stored: Vec<(UserId, String)> = self
            .lock()?
            .map(kind)
            .iter()
            .map(|(user, hash)| (*user, hash.clone()))
            .collect();
        let mut found = None;
        for (user, stored) in stored {
            if let Some(bytes) = unhex(&stored)
                && ShareTokenHash::from_bytes(bytes) == hash
            {
                found = Some(user);
            }
        }
        Ok(found)
    }
}

/// Slack users linked to Vellum handles, kept in `slack-users.json` as
/// `{"<team>/<user>": "<handle>"}`. People link themselves from the "Save
/// as task" shortcut (#163); an admin may also edit the file.
#[derive(Debug)]
struct SlackLinks {
    path: Option<PathBuf>,
    links: Mutex<HashMap<String, String>>,
}

impl SlackLinks {
    fn open(dir: Option<&Path>) -> Result<Self, Error> {
        let path = dir.map(|d| d.join("slack-users.json"));
        let links = match &path {
            Some(p) if p.exists() => {
                let bytes =
                    std::fs::read(p).map_err(|e| unavailable("cannot read Slack links", e))?;
                serde_json::from_slice(&bytes)
                    .map_err(|e| unavailable("Slack links are not valid JSON", e))?
            }
            _ => HashMap::new(),
        };
        Ok(Self {
            path,
            links: Mutex::new(links),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn get(&self, key: &str) -> Option<String> {
        self.lock().get(key).cloned()
    }

    /// Changes the links and saves them.
    fn change<R>(&self, f: impl FnOnce(&mut HashMap<String, String>) -> R) -> Result<R, Error> {
        let (out, bytes) = {
            let mut links = self.lock();
            let out = f(&mut links);
            let sorted: BTreeMap<&String, &String> = links.iter().collect();
            let bytes = serde_json::to_vec_pretty(&sorted)
                .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
            drop(links);
            (out, bytes)
        };
        let Some(path) = &self.path else {
            return Ok(out);
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| unavailable("cannot save Slack links", e))?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|e| unavailable("cannot save Slack links", e))?;
        std::fs::rename(&temp, path).map_err(|e| unavailable("cannot save Slack links", e))?;
        Ok(out)
    }
}

/// Posts Slack replies; only to `https://hooks.slack.com/`, never
/// following redirects, within Slack's three-second window.
fn slack_client() -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(2500))
        .build()
        .map_err(|e| {
            Error::new(
                ErrorKind::Internal,
                format!("cannot build the Slack client: {e}"),
            )
        })
}

/// Settings for [`DriveTasks`].
#[derive(Debug, Clone, Default)]
pub struct TasksSettings {
    /// Where secrets and Slack links are kept (`<data>/.vellum/tasks`);
    /// `None` keeps secrets in memory.
    pub dir: Option<PathBuf>,
    /// The time zone "today" is counted in.
    pub time_zone: Option<TimeZone>,
}

/// My Work's view of the server.
pub struct DriveTasks<S> {
    drive: Arc<Drive>,
    store: Arc<dyn BlobStore>,
    index: Arc<dyn DriveIndex>,
    authz: Arc<Authz<S>>,
    identity: Arc<Identity>,
    work: Arc<WorkService>,
    secrets: Secrets,
    people: PeoplePrefs,
    slack: SlackLinks,
    slack_http: reqwest::Client,
    time_zone: TimeZone,
}

impl<S> std::fmt::Debug for DriveTasks<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveTasks").finish_non_exhaustive()
    }
}

/// The pieces [`DriveTasks`] works over.
pub struct Parts<S> {
    /// The drive.
    pub drive: Arc<Drive>,
    /// The files under it.
    pub store: Arc<dyn BlobStore>,
    /// The drive's index, for space owners.
    pub index: Arc<dyn DriveIndex>,
    /// The permission engine, for sharing lists.
    pub authz: Arc<Authz<S>>,
    /// People and handles.
    pub identity: Arc<Identity>,
    /// Issues and comments.
    pub work: Arc<WorkService>,
}

impl<S: AuthzStore + 'static> DriveTasks<S> {
    /// Opens the adapter.
    ///
    /// # Errors
    ///
    /// An unreadable secrets or Slack links file.
    pub fn open(parts: Parts<S>, settings: &TasksSettings) -> Result<Self, Error> {
        let dir = settings.dir.as_deref();
        Ok(Self {
            drive: parts.drive,
            store: parts.store,
            index: parts.index,
            authz: parts.authz,
            identity: parts.identity,
            work: parts.work,
            secrets: Secrets::open(dir)?,
            people: PeoplePrefs::open(dir)?,
            slack: SlackLinks::open(dir)?,
            slack_http: slack_client()?,
            time_zone: settings.time_zone.clone().unwrap_or_else(TimeZone::system),
        })
    }

    fn handle_of(&self, user: UserId) -> Result<String, Error> {
        Ok(self.identity.user(user)?.handle)
    }

    async fn my_drive(&self, user: UserId, create: bool) -> Result<Option<Space>, Error> {
        if let Some(space) = self.drive.my_drive(user).await? {
            return Ok(Some(space));
        }
        if !create {
            return Ok(None);
        }
        let person = self.identity.user(user)?;
        let space = self
            .drive
            .create_space(
                &Actor::User { user },
                SpaceKind::UserDrive,
                &person.handle,
                &person.name,
                Some(user),
            )
            .await?;
        Ok(Some(space))
    }

    /// The `Tasks` folder in a My Drive, created when missing.
    async fn tasks_folder(&self, user: UserId, space: &Space) -> Result<ObjectId, Error> {
        let actor = Actor::User { user };
        let top = self.drive.list(&actor, space.id, None).await?;
        if let Some(folder) = top
            .iter()
            .find(|n| n.kind == NodeKind::Folder && n.title == TASKS_FOLDER)
        {
            return Ok(folder.id);
        }
        Ok(self
            .drive
            .create_folder(&actor, space.id, None, TASKS_FOLDER)
            .await?
            .id)
    }

    /// Whether a page is a list: directly in the `Tasks` folder of a My
    /// Drive. Returns the owner's handle. `spaces` caches each space's
    /// owner and `Tasks` folder.
    async fn list_owner(
        &self,
        node: &Node,
        spaces: &mut HashMap<SpaceId, Option<(String, BlobPath)>>,
    ) -> Result<Option<String>, Error> {
        if let std::collections::hash_map::Entry::Vacant(slot) = spaces.entry(node.space) {
            let entry = match self.index.space(node.space).await? {
                Some(space) if space.kind == SpaceKind::UserDrive => {
                    let folder = vellum_drive::layout::file_name(NodeKind::Folder, TASKS_FOLDER, 1);
                    match (space.owner, space.root.join(&folder)) {
                        (Some(owner), Ok(tasks)) => {
                            self.handle_of(owner).ok().map(|handle| (handle, tasks))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let _ = slot.insert(entry);
        }
        Ok(spaces
            .get(&node.space)
            .and_then(Option::as_ref)
            .filter(|(_, tasks)| node.path.parent().as_ref() == Some(tasks))
            .map(|(handle, _)| handle.clone()))
    }

    async fn load(&self, node: &Node, context: FileContext) -> Result<Document, Error> {
        let blob = self
            .store
            .get(&node.path)
            .await
            .map_err(|e| unavailable("cannot read a task file", e))?;
        Ok(Document {
            context,
            text: String::from_utf8_lossy(&blob.bytes).into_owned(),
            version: version_of(&blob.meta.etag),
            modified: to_jiff(node.modified),
        })
    }

    async fn files(&self, user: UserId) -> Result<Vec<Document>, Error> {
        let handle = self.handle_of(user)?;
        let mention = format!("@{handle}");
        let filter = Filter {
            kinds: vec![NodeKind::Page],
            ..Filter::default()
        };
        let pages = self
            .drive
            .find(&Actor::User { user }, &filter, FIND_LIMIT)
            .await?;
        let mut owners = HashMap::new();
        let mut out = Vec::new();
        let mut scanned = 0;
        for node in pages {
            let doc = DocId::from(node.id);
            if let Some(owner) = self.list_owner(&node, &mut owners).await? {
                let origin = if node.title == INBOX {
                    Origin::Inbox
                } else {
                    Origin::List
                };
                let context = FileContext {
                    doc,
                    origin,
                    owner: Some(owner),
                    list: Some(node.title.clone()),
                };
                out.push(self.load(&node, context).await?);
                continue;
            }
            if scanned >= DOC_SCAN {
                continue;
            }
            scanned += 1;
            let context = FileContext {
                doc,
                origin: Origin::Doc,
                owner: None,
                list: None,
            };
            let file = self.load(&node, context).await?;
            if file.text.contains(&mention) {
                out.push(file);
            }
        }
        Ok(out)
    }

    async fn assigned_to(&self, user: UserId) -> Result<Vec<Task>, Error> {
        let handle = self.handle_of(user)?;
        let Ok(me) = UserRef::parse(&handle) else {
            return Ok(Vec::new());
        };
        let dismissed = self.people.dismissed(user);
        let tasks = self
            .work
            .read(|graph, comments| {
                let mut out = Vec::new();
                for issue in graph.issues() {
                    if issue.assignee.as_ref() != Some(&me) {
                        continue;
                    }
                    let Some(category) = graph.category_of(issue).map(category) else {
                        continue;
                    };
                    if matches!(
                        category,
                        IssueCategory::Completed
                            | IssueCategory::Canceled
                            | IssueCategory::Duplicate
                    ) {
                        continue;
                    }
                    let fields = TaskFields {
                        due: issue.due.and_then(|d| {
                            jiff::civil::Date::new(
                                i16::try_from(d.year()).ok()?,
                                i8::try_from(u8::from(d.month())).ok()?,
                                i8::try_from(d.day()).ok()?,
                            )
                            .ok()
                            .map(Due::Date)
                        }),
                        priority: Priority::new(issue.priority.number()),
                        ..TaskFields::default()
                    };
                    out.push(Task {
                        source: Source::Issue {
                            issue: issue.id,
                            key: issue.key().to_string(),
                            category,
                        },
                        title: issue.title.clone(),
                        done: false,
                        fields,
                        assignees: vec![handle.clone()],
                        parent: None,
                        list: None,
                    });
                }
                out.extend(mention_tasks(graph, comments, &me, &handle, &dismissed));
                for assignment in comments.assignments(&me) {
                    if let Target::Doc { doc, .. } = assignment.target {
                        out.push(Task {
                            source: Source::Comment {
                                comment: assignment.thread,
                                doc,
                                resolved: false,
                            },
                            title: assignment.summary,
                            done: false,
                            fields: TaskFields::default(),
                            assignees: vec![handle.clone()],
                            parent: None,
                            list: None,
                        });
                    }
                }
                out
            })
            .await;
        Ok(self.readable_only(user, tasks).await)
    }

    /// Drops mentions on documents the person cannot open; mentioning
    /// someone does not share the document with them.
    async fn readable_only(&self, user: UserId, tasks: Vec<Task>) -> Vec<Task> {
        let actor = Actor::User { user };
        let mut open = Vec::with_capacity(tasks.len());
        let mut readable: HashMap<DocId, bool> = HashMap::new();
        for task in tasks {
            if let Source::Mention { doc: Some(doc), .. } = &task.source {
                let can = if let Some(can) = readable.get(doc) {
                    *can
                } else {
                    let can = self
                        .drive
                        .get(&actor, (*doc).into())
                        .await
                        .is_ok_and(|node| node.is_live());
                    let _ = readable.insert(*doc, can);
                    can
                };
                if !can {
                    continue;
                }
            }
            open.push(task);
        }
        open
    }

    async fn read_doc(&self, user: UserId, doc: DocId) -> Result<Document, Error> {
        let actor = Actor::User { user };
        let node = self.drive.get(&actor, doc.into()).await?;
        if node.kind != NodeKind::Page || !node.is_live() {
            return Err(Error::new(ErrorKind::NotFound, "no such task file"));
        }
        let mut owners = HashMap::new();
        let owner = self.list_owner(&node, &mut owners).await?;
        let context = match owner {
            Some(owner) => FileContext {
                doc,
                origin: if node.title == INBOX {
                    Origin::Inbox
                } else {
                    Origin::List
                },
                owner: Some(owner),
                list: Some(node.title.clone()),
            },
            None => FileContext {
                doc,
                origin: Origin::Doc,
                owner: None,
                list: None,
            },
        };
        self.load(&node, context).await
    }

    async fn write_doc(
        &self,
        user: UserId,
        doc: DocId,
        text: String,
        version: u64,
    ) -> Result<u64, Error> {
        let actor = Actor::User { user };
        let node = self.drive.get(&actor, doc.into()).await?;
        let current = self
            .store
            .get(&node.path)
            .await
            .map_err(|e| unavailable("cannot read a task file", e))?;
        if version_of(&current.meta.etag) != version {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the file changed since it was read",
            ));
        }
        let base = Base {
            bytes: current.bytes,
            etag: Some(current.meta.etag),
        };
        let outcome = self
            .drive
            .save(&actor, node.id, base, Bytes::from(text))
            .await?;
        if let Saved::Conflicted { .. } = outcome.saved {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the file changed while it was being saved",
            ));
        }
        let meta = self
            .store
            .head(&outcome.node.path)
            .await
            .map_err(|e| unavailable("cannot read a task file", e))?;
        Ok(version_of(&meta.etag))
    }

    async fn create_list(&self, user: UserId, text: String) -> Result<DocId, Error> {
        let space = self
            .my_drive(user, true)
            .await?
            .ok_or_else(|| Error::new(ErrorKind::Internal, "no My Drive"))?;
        let folder = self.tasks_folder(user, &space).await?;
        let title = vellum_tasks::lists::read_meta(&text).title;
        let actor = Actor::User { user };
        let node = self
            .drive
            .create_page(&actor, space.id, Some(folder), &title, "")
            .await?;
        let created = self
            .store
            .get(&node.path)
            .await
            .map_err(|e| unavailable("cannot read a new list", e))?;
        let text = vellum_drive::frontmatter::set_md(&text, "id", &node.id.to_string());
        let base = Base {
            bytes: created.bytes,
            etag: Some(created.meta.etag),
        };
        let _ = self
            .drive
            .save(&actor, node.id, base, Bytes::from(text))
            .await?;
        Ok(DocId::from(node.id))
    }

    async fn share(
        &self,
        user: UserId,
        list: DocId,
        handle: &str,
        member: bool,
    ) -> Result<(), Error> {
        let person = self.identity.user_by_handle(handle).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("no one is called @{handle}"))
        })?;
        let node = self.drive.get(&Actor::User { user }, list.into()).await?;
        let object = Object::Node(Id(node.id.as_u128()));
        let subject = Subject::User(Id(person.id.as_u128()));
        let change = if member {
            Change::Grant(Tuple {
                object,
                relation: TupleRelation::Role(Relation::Editor),
                subject,
                expires_at: None,
            })
        } else {
            Change::Revoke {
                object,
                relation: Relation::Editor,
                subject,
            }
        };
        let _ = self
            .authz
            .write(
                Id(node.space.as_u128()),
                AuthzActor::User(Id(user.as_u128())),
                &[change],
            )
            .await
            .map_err(|e| Error::new(ErrorKind::PermissionDenied, e.to_string()))?;
        Ok(())
    }
}

/// Unanswered, undismissed mentions of `me` as My Work items.
fn mention_tasks(
    graph: &vellum_work::WorkGraph,
    comments: &vellum_work::Comments,
    me: &UserRef,
    handle: &str,
    dismissed: &[CommentId],
) -> Vec<Task> {
    let mut out = Vec::new();
    for mention in comments.mentions_of(me) {
        if dismissed.contains(&mention.comment) {
            continue;
        }
        let (doc, issue) = match mention.target {
            Target::Doc { doc, .. } => (Some(doc), None),
            Target::Issue { issue } => {
                let Some(found) = graph.issue(&issue) else {
                    continue;
                };
                (None, Some(found.key().to_string()))
            }
        };
        out.push(Task {
            source: Source::Mention {
                comment: mention.comment,
                doc,
                issue,
                by: mention.author.to_string(),
            },
            title: mention.summary,
            done: false,
            fields: TaskFields::default(),
            assignees: vec![handle.to_owned()],
            parent: None,
            list: None,
        });
    }
    out
}

const fn category(category: Category) -> IssueCategory {
    match category {
        Category::Triage => IssueCategory::Triage,
        Category::Backlog => IssueCategory::Backlog,
        Category::Unstarted => IssueCategory::Unstarted,
        Category::Started => IssueCategory::Started,
        Category::Completed => IssueCategory::Completed,
        Category::Canceled => IssueCategory::Canceled,
        Category::Duplicate => IssueCategory::Duplicate,
    }
}

impl<S: AuthzStore + 'static> Workspace for DriveTasks<S> {
    fn profile(&self, user: UserId) -> BoxFuture<'_, Result<Profile, Error>> {
        Box::pin(async move {
            let person = self.identity.user(user)?;
            let preferences = self.people.preferences(user);
            Ok(Profile {
                user,
                handle: person.handle,
                display_name: person.name,
                time_zone: preferences.zone_or(&self.time_zone),
                preferences,
            })
        })
    }

    fn task_files(&self, user: UserId) -> BoxFuture<'_, Result<Vec<Document>, Error>> {
        Box::pin(self.files(user))
    }

    fn assigned(&self, user: UserId) -> BoxFuture<'_, Result<Vec<Task>, Error>> {
        Box::pin(self.assigned_to(user))
    }

    fn read(&self, user: UserId, doc: DocId) -> BoxFuture<'_, Result<Document, Error>> {
        Box::pin(self.read_doc(user, doc))
    }

    fn write(
        &self,
        user: UserId,
        doc: DocId,
        text: String,
        version: u64,
    ) -> BoxFuture<'_, Result<u64, Error>> {
        Box::pin(self.write_doc(user, doc, text, version))
    }

    fn create(
        &self,
        user: UserId,
        _path: String,
        text: String,
    ) -> BoxFuture<'_, Result<DocId, Error>> {
        Box::pin(self.create_list(user, text))
    }

    fn set_member(
        &self,
        user: UserId,
        list: DocId,
        handle: String,
        member: bool,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.share(user, list, &handle, member).await })
    }

    fn set_secret(
        &self,
        user: UserId,
        kind: SecretKind,
        hash: Option<ShareTokenHash>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.secrets.set(user, kind, hash) })
    }

    fn user_for_secret(
        &self,
        kind: SecretKind,
        hash: ShareTokenHash,
    ) -> BoxFuture<'_, Result<Option<UserId>, Error>> {
        Box::pin(async move { self.secrets.user_for(kind, hash) })
    }

    fn save_preferences(
        &self,
        user: UserId,
        preferences: Preferences,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.people.set_preferences(user, &preferences) })
    }

    fn dismiss_mention(
        &self,
        user: UserId,
        comment: CommentId,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { self.people.dismiss(user, comment) })
    }

    fn people(&self) -> BoxFuture<'_, Result<Vec<UserId>, Error>> {
        Box::pin(async move {
            Ok(self
                .identity
                .users()
                .into_iter()
                .filter(|u| !u.disabled)
                .map(|u| u.id)
                .collect())
        })
    }

    fn slack_user(
        &self,
        team: String,
        slack_user: String,
    ) -> BoxFuture<'_, Result<Option<UserId>, Error>> {
        Box::pin(async move {
            Ok(self
                .slack
                .get(&format!("{team}/{slack_user}"))
                .and_then(|handle| self.identity.user_by_handle(&handle))
                .map(|person| person.id))
        })
    }

    fn link_slack(
        &self,
        user: UserId,
        team: String,
        slack_user: String,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let handle = self.handle_of(user)?;
            self.slack.change(|links| {
                let _ = links.insert(format!("{team}/{slack_user}"), handle);
            })
        })
    }

    fn unlink_slack(&self, user: UserId) -> BoxFuture<'_, Result<usize, Error>> {
        Box::pin(async move {
            let handle = self.handle_of(user)?;
            self.slack.change(|links| {
                let before = links.len();
                links.retain(|_, h| *h != handle);
                before - links.len()
            })
        })
    }

    fn slack_say(&self, response_url: String, text: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if !response_url.starts_with(vellum_tasks::capture::SLACK_RESPONSE_PREFIX) {
                return;
            }
            let sent = self
                .slack_http
                .post(&response_url)
                .json(&serde_json::json!({ "response_type": "ephemeral", "text": text }))
                .send()
                .await;
            if let Err(error) = sent {
                tracing::warn!(%error, "Slack reply not sent");
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_stable_and_distinct() {
        let a = Etag::new("abc");
        assert_eq!(version_of(&a), version_of(&Etag::new("abc")));
        assert_ne!(version_of(&a), version_of(&Etag::new("abd")));
    }

    #[test]
    fn hashes_round_trip_through_hex() {
        let bytes = [7_u8; 32];
        assert_eq!(unhex(&hex(&bytes)), Some(bytes));
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn secrets_persist_and_match_only_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let user = UserId::generate();
        let hash = ShareTokenHash::from_bytes([9; 32]);
        let secrets = Secrets::open(Some(dir.path())).unwrap();
        secrets.set(user, SecretKind::Feed, Some(hash)).unwrap();
        let reopened = Secrets::open(Some(dir.path())).unwrap();
        assert_eq!(
            reopened.user_for(SecretKind::Feed, hash).unwrap(),
            Some(user)
        );
        assert_eq!(reopened.user_for(SecretKind::Capture, hash).unwrap(), None);
        reopened.set(user, SecretKind::Feed, None).unwrap();
        assert_eq!(reopened.user_for(SecretKind::Feed, hash).unwrap(), None);
    }
}
