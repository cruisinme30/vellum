// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The My Work service: the typed API the gateway turns into REST
//! routes, OpenAPI operations and `GraphQL` fields (R-MW-1 to R-MW-8).
//!
//! Every operation works on Markdown through the [`Workspace`] port: the
//! checklist block is the task, so completing, moving or editing a task
//! is a text edit to the file it lives in, written back with the version
//! it was read at. The calendar feed, CalDAV and capture endpoints are
//! not JSON and live in [`crate::raw`].

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope, Service};
use vellum_kernel::{BlockId, CommentId, DocId, Error, ErrorKind, UserId};

use crate::attrs::Attrs;
use crate::checklist::{self, Checklist, Completion, EditError};
use crate::import;
use crate::lists;
use crate::model::{Origin, Source, Task, TaskFields};
use crate::port::{Document, Profile, SecretKind, Workspace};
use crate::projection::{self, Entry, MyWork, Target, UpcomingGroup};
use crate::quickadd::{self, TokenKind};
use crate::remind;
use crate::settings::{MyWorkSettingsInput, MyWorkSettingsView, Preferences};
use crate::{caldav, capture, feed};

/// How many completed tasks the view shows.
pub const DONE_SHOWN: usize = 50;

/// How often an edit is retried when the file changed underneath it.
const EDIT_ATTEMPTS: usize = 3;

/// Deployment settings for the service.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// The public URL of the web app, without a trailing `/`, for feed
    /// URLs and links back.
    pub base_url: String,
    /// The mail domain that receives capture e-mail.
    pub mail_domain: String,
    /// The Slack app's signing secret, when Slack capture is set up.
    pub slack_signing_secret: Option<Vec<u8>>,
}

/// The source of the current time; tests pin it.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The My Work service.
pub struct TasksService {
    pub(crate) workspace: Arc<dyn Workspace>,
    pub(crate) config: Config,
    clock: Clock,
    pub(crate) google: Option<Arc<crate::google::Google>>,
    /// People whose task files changed through this service since the
    /// reminder scheduler last asked.
    edited: Arc<Mutex<BTreeSet<UserId>>>,
}

impl std::fmt::Debug for TasksService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TasksService")
            .field("base_url", &self.config.base_url)
            .finish_non_exhaustive()
    }
}

// --- Errors -------------------------------------------------------------

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

fn not_found(message: &'static str) -> Error {
    Error::new(ErrorKind::NotFound, message)
}

pub(crate) fn edit_error(err: EditError) -> Error {
    Error::new(ErrorKind::NotFound, "no such task").with_source(err)
}

// --- Views --------------------------------------------------------------

/// A task's fields as strings, in the attribute-span syntax (D82).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FieldsView {
    /// `evening`, `anytime` or `someday`.
    pub when: Option<String>,
    /// Start date, `YYYY-MM-DD`.
    pub start: Option<String>,
    /// Deadline, `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`.
    pub due: Option<String>,
    /// RFC 5545 repeat rule, without `RRULE:`.
    pub rrule: Option<String>,
    /// `completion` when the next date counts from completion.
    pub recur_from: Option<String>,
    /// Reminders: ISO durations relative to the deadline, or local times.
    pub remind: Vec<String>,
    /// Priority 1 (highest) to 4.
    pub priority: Option<u8>,
    /// Labels.
    pub labels: Vec<String>,
    /// When it was completed (UTC).
    pub done_at: Option<String>,
    /// A time block: local start and minutes, `2026-11-03T15:00/90`.
    pub block: Option<String>,
}

impl FieldsView {
    fn new(fields: &TaskFields) -> Self {
        let mut attrs = Attrs::new();
        fields.write_attrs(&mut attrs);
        let text = |key: &str| attrs.get(key).map(str::to_owned);
        let many = |key: &str| {
            attrs
                .get(key)
                .map(|v| v.split(',').map(str::to_owned).collect())
                .unwrap_or_default()
        };
        Self {
            when: text("when"),
            start: text("start"),
            due: text("due"),
            rrule: text("rrule"),
            recur_from: text("recur_from"),
            remind: many("remind"),
            priority: fields.priority.map(crate::model::Priority::level),
            labels: many("labels"),
            done_at: text("done"),
            block: text("block"),
        }
    }
}

/// Where a task comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// A checklist item in a list or document.
    Checklist,
    /// An issue assigned to the person.
    Issue,
    /// A comment that assigns the person.
    Comment,
    /// A comment that mentions the person.
    Mention,
    /// A review request.
    Review,
}

/// One task in a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TaskView {
    /// The task ID: `<doc>.<block>` for checklist items.
    pub id: String,
    /// Where it comes from.
    pub kind: TaskKind,
    /// The title, as Vellum Markdown inline text.
    pub title: String,
    /// Whether it is completed.
    pub done: bool,
    /// Whether its deadline has passed.
    pub overdue: bool,
    /// The document it lives in, for checklist items and comments.
    pub doc: Option<DocId>,
    /// The list's title, for items in task lists.
    pub list: Option<String>,
    /// The issue key, for issues.
    pub issue_key: Option<String>,
    /// The fields.
    pub fields: FieldsView,
    /// People mentioned in the item, without `@`.
    pub assignees: Vec<String>,
    /// For a mention: who mentioned the person, without `@`.
    pub mentioned_by: Option<String>,
    /// Subtasks.
    pub subtasks: Vec<Self>,
}

impl TaskView {
    fn new(task: &Task, overdue: bool, subtasks: Vec<Self>) -> Self {
        let (kind, doc, issue_key) = match &task.source {
            Source::Checklist { doc, .. } => (TaskKind::Checklist, Some(*doc), None),
            Source::Issue { key, .. } => (TaskKind::Issue, None, Some(key.clone())),
            Source::Comment { doc, .. } => (TaskKind::Comment, Some(*doc), None),
            Source::Mention { doc, issue, .. } => (TaskKind::Mention, *doc, issue.clone()),
            Source::Review { .. } => (TaskKind::Review, None, None),
        };
        Self {
            id: task.source.key(),
            kind,
            title: task.title.clone(),
            done: task.done,
            overdue,
            doc,
            list: task.list.clone(),
            issue_key,
            fields: FieldsView::new(&task.fields),
            assignees: task.assignees.clone(),
            mentioned_by: match &task.source {
                Source::Mention { by, .. } => Some(by.clone()),
                _ => None,
            },
            subtasks,
        }
    }

    fn from_entry(entry: &Entry) -> Self {
        let subtasks = entry.children.iter().map(Self::from_entry).collect();
        Self::new(&entry.task, entry.overdue, subtasks)
    }

    fn standalone(task: &Task, today: Date) -> Self {
        let overdue = !task.done && task.fields.due.is_some_and(|d| d.date() < today);
        Self::new(task, overdue, Vec::new())
    }
}

/// A bucket of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BucketName {
    /// Not yet sorted.
    Inbox,
    /// Today, overdue first.
    Today,
    /// This evening.
    Evening,
    /// A later day or week.
    Upcoming,
    /// Ready whenever.
    Anytime,
    /// Parked.
    Someday,
    /// Completed, most recent first.
    Done,
}

/// One section of My Work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SectionView {
    /// The bucket.
    pub bucket: BucketName,
    /// For Upcoming: the day (`YYYY-MM-DD`) within the next seven.
    pub day: Option<String>,
    /// For Upcoming: the Monday of a later week.
    pub week_of: Option<String>,
    /// The tasks, in display order.
    pub tasks: Vec<TaskView>,
}

/// My Work for the signed-in person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MyWorkView {
    /// The person's local date.
    pub today: String,
    /// How many open tasks the view shows.
    pub open: usize,
    /// The sections in order; empty ones are left out.
    pub sections: Vec<SectionView>,
}

impl MyWorkView {
    fn new(view: &MyWork, today: Date) -> Self {
        let mut sections = Vec::new();
        let mut push = |bucket, day, week_of, entries: &[Entry]| {
            if !entries.is_empty() {
                sections.push(SectionView {
                    bucket,
                    day,
                    week_of,
                    tasks: entries.iter().map(TaskView::from_entry).collect(),
                });
            }
        };
        push(BucketName::Inbox, None, None, &view.inbox);
        push(BucketName::Today, None, None, &view.today);
        push(BucketName::Evening, None, None, &view.evening);
        for (group, entries) in &view.upcoming {
            match group {
                UpcomingGroup::Day(day) => {
                    push(BucketName::Upcoming, Some(day.to_string()), None, entries);
                }
                UpcomingGroup::Week(monday) => {
                    push(
                        BucketName::Upcoming,
                        None,
                        Some(monday.to_string()),
                        entries,
                    );
                }
            }
        }
        push(BucketName::Anytime, None, None, &view.anytime);
        push(BucketName::Someday, None, None, &view.someday);
        let done: Vec<Entry> = view.done.iter().take(DONE_SHOWN).cloned().collect();
        push(BucketName::Done, None, None, &done);
        Self {
            today: today.to_string(),
            open: view.open_count(),
            sections,
        }
    }
}

/// What a recognised quick-add phrase set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenView {
    /// The deadline.
    Due,
    /// The start date.
    Start,
    /// A repeat rule.
    Recurrence,
    /// Someday, Anytime or This evening.
    When,
    /// `#list`.
    List,
    /// `@label`.
    Label,
    /// `+person`.
    Person,
    /// `p1` to `p4`.
    Priority,
    /// A reminder.
    Reminder,
}

impl From<TokenKind> for TokenView {
    fn from(kind: TokenKind) -> Self {
        match kind {
            TokenKind::Due => Self::Due,
            TokenKind::Start => Self::Start,
            TokenKind::Recurrence => Self::Recurrence,
            TokenKind::When => Self::When,
            TokenKind::List => Self::List,
            TokenKind::Label => Self::Label,
            TokenKind::Person => Self::Person,
            TokenKind::Priority => Self::Priority,
            TokenKind::Reminder => Self::Reminder,
        }
    }
}

/// A phrase to highlight in the quick-add box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct HighlightView {
    /// Start, in UTF-16 code units (`JavaScript` string indices).
    pub start: usize,
    /// End, exclusive, in UTF-16 code units.
    pub end: usize,
    /// What it set.
    pub kind: TokenView,
}

/// A parsed quick-add line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ParseView {
    /// The title without the recognised phrases.
    pub title: String,
    /// The fields the phrases set.
    pub fields: FieldsView,
    /// The list named with `#list`.
    pub list: Option<String>,
    /// People named with `+person`.
    pub assignees: Vec<String>,
    /// Phrases to highlight.
    pub highlights: Vec<HighlightView>,
}

/// A task list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListView {
    /// The list's document.
    pub doc: DocId,
    /// The title.
    pub title: String,
    /// Whether it is the person's Inbox.
    pub inbox: bool,
    /// The owner's handle.
    pub owner: Option<String>,
    /// Members' handles.
    pub members: Vec<String>,
    /// How many open items it holds.
    pub open: usize,
}

/// The person's task lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListsView {
    /// Inbox first, then by title.
    pub lists: Vec<ListView>,
}

/// The result of completing a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CompleteView {
    /// The task after the change.
    pub task: TaskView,
    /// For a repeating task: the date it moved to. It stays open.
    pub next: Option<String>,
}

/// A removed task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DeletedView {
    /// Its ID.
    pub id: String,
}

/// One list written by an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ImportedList {
    /// The list's document.
    pub doc: DocId,
    /// Its title.
    pub title: String,
    /// How many items were added; items already imported are skipped.
    pub added: usize,
}

/// The result of an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ImportView {
    /// The lists written.
    pub lists: Vec<ImportedList>,
}

/// How the person's tasks reach other apps, for My Work's settings (D298).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConnectionsView {
    /// The CalDAV account URL to give a task app (it signs in with an API token).
    pub caldav_url: String,
    /// Whether the server receives capture e-mail (a mail domain is set).
    pub email: bool,
    /// Whether Slack capture is set up on the server.
    pub slack: bool,
    /// Whether this person is linked to a Slack user.
    pub slack_linked: bool,
}

/// A newly issued secret URL or address. It is shown once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SecretView {
    /// The feed URL or capture address.
    pub value: String,
}

/// An acknowledgement with nothing to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
pub struct Done {}

/// The daily digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DigestView {
    /// Vellum Markdown, or nothing when the day is clear.
    pub markdown: Option<String>,
}

// --- Inputs -------------------------------------------------------------

/// No input.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
pub struct NoInput {}

/// A code from Slack's "Save as task" reply.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SlackLinkInput {
    /// The `slack_link` value from the link Slack showed.
    pub code: String,
}

/// A quick-add line to parse.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ParseInput {
    /// The line as typed.
    pub text: String,
}

/// A new task.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateInput {
    /// The quick-add line: title plus phrases such as `tomorrow 3pm`,
    /// `every weekday`, `#list`, `@label`, `+person`, `p1`.
    pub text: String,
    /// The list to add to when the line names none; the Inbox otherwise.
    pub list: Option<String>,
    /// A task ID to add a subtask under.
    pub parent: Option<String>,
}

/// A mention by the ID of the comment that made it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MentionRef {
    /// The comment ID.
    pub comment: String,
}

/// A task by ID.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TaskRef {
    /// The task ID.
    pub task: String,
}

/// Completing a task.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CompleteInput {
    /// The task ID.
    pub task: String,
    /// Also complete its open subtasks.
    #[serde(default)]
    pub cascade: bool,
}

/// Dragging a task to another bucket.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MoveInput {
    /// The task ID.
    pub task: String,
    /// `inbox`, `today`, `evening`, `anytime`, `someday` or a date
    /// (`YYYY-MM-DD`). Moves the start date; the deadline stays.
    pub to: String,
}

/// Changes to a task. Each field present replaces the old value; an
/// empty string clears it.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct UpdateInput {
    /// The task ID.
    pub task: String,
    /// The new title.
    pub title: Option<String>,
    /// `evening`, `anytime` or `someday`.
    pub when: Option<String>,
    /// `YYYY-MM-DD`.
    pub start: Option<String>,
    /// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`.
    pub due: Option<String>,
    /// An RFC 5545 rule such as `FREQ=WEEKLY;BYDAY=MO`.
    pub rrule: Option<String>,
    /// `schedule` or `completion`.
    pub recur_from: Option<String>,
    /// Comma-separated reminders, such as `-PT15M,2026-11-03T09:00`.
    pub remind: Option<String>,
    /// `1` to `4`.
    pub priority: Option<String>,
    /// Comma-separated labels.
    pub labels: Option<String>,
    /// A time block, `YYYY-MM-DDTHH:MM/<minutes>`, shown in Google
    /// Calendar when it is connected.
    pub block: Option<String>,
}

/// A new list.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateListInput {
    /// Its title.
    pub title: String,
}

/// Sharing a list.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MemberInput {
    /// The list's document.
    pub list_id: DocId,
    /// The person's handle, with or without `@`.
    pub member: String,
}

/// Which app an export comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportFrom {
    /// Google Takeout `Tasks.json`, or the Tasks API's lists.
    GoogleTasks,
    /// A Todoist project exported as CSV.
    Todoist,
    /// Things 3 JSON (the `things:///json` schema).
    Things,
}

/// An export to import.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ImportInput {
    /// The app.
    pub from: ImportFrom,
    /// The export's text.
    pub content: String,
    /// The list title for a Todoist CSV, which does not name its project.
    pub project: Option<String>,
}

// --- Helpers ------------------------------------------------------------

/// Splits a checklist task ID, `<doc>.<block>`.
pub(crate) fn parse_task_id(id: &str) -> Result<(DocId, BlockId), Error> {
    let (doc, block) = id
        .split_once('.')
        .ok_or_else(|| invalid("only checklist tasks can be changed here"))?;
    let doc = doc.parse().map_err(|_| invalid("not a task ID"))?;
    let block = block.parse().map_err(|_| invalid("not a task ID"))?;
    Ok((doc, block))
}

fn parse_target(to: &str) -> Result<Target, Error> {
    Ok(match to.trim().to_ascii_lowercase().as_str() {
        "inbox" => Target::Inbox,
        "today" => Target::Today,
        "evening" | "this evening" => Target::Evening,
        "anytime" => Target::Anytime,
        "someday" => Target::Someday,
        other => Target::Day(
            other
                .parse()
                .map_err(|_| invalid("move to a bucket or a YYYY-MM-DD date"))?,
        ),
    })
}

fn utf16_offset(text: &str, byte: usize) -> usize {
    text.get(..byte).map_or(0, |s| s.encode_utf16().count())
}

fn with_mentions(title: &str, people: &[String]) -> String {
    let mut title = title.to_owned();
    for person in people {
        let mention = format!("@{person}");
        if !checklist::mentions(&title).iter().any(|m| m == person) {
            title.push(' ');
            title.push_str(&mention);
        }
    }
    title
}

fn find_task(doc: &Document, block: BlockId) -> Option<Task> {
    projection::project_file(&doc.context, &doc.text)
        .into_iter()
        .find(|t| matches!(&t.source, Source::Checklist { block: b, .. } if *b == block))
}

const fn is_list(doc: &Document) -> bool {
    matches!(doc.context.origin, Origin::Inbox | Origin::List)
}

fn list_title(doc: &Document) -> String {
    doc.context
        .list
        .clone()
        .unwrap_or_else(|| lists::read_meta(&doc.text).title)
}

/// The signed-in person and their local time now.
pub(crate) struct Me {
    pub(crate) profile: Profile,
    pub(crate) now: Zoned,
}

impl Me {
    pub(crate) fn today(&self) -> Date {
        self.now.date()
    }
}

// --- The service --------------------------------------------------------

impl TasksService {
    /// A service over `workspace`.
    #[must_use]
    pub fn new(workspace: Arc<dyn Workspace>, config: Config) -> Self {
        Self {
            workspace,
            config,
            clock: Arc::new(Timestamp::now),
            google: None,
            edited: Arc::default(),
        }
    }

    /// Replaces the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub(crate) fn now(&self) -> Timestamp {
        (self.clock)()
    }

    /// Notes that `user`'s tasks may have changed, so their reminders are
    /// planned again on the scheduler's next tick.
    pub(crate) fn touch(&self, user: UserId) {
        if let Ok(mut edited) = self.edited.lock() {
            let _ = edited.insert(user);
        }
    }

    /// The people whose tasks changed through this service since the last
    /// call.
    #[must_use]
    pub fn take_edited(&self) -> Vec<UserId> {
        self.edited
            .lock()
            .map(|mut edited| std::mem::take(&mut *edited).into_iter().collect())
            .unwrap_or_default()
    }

    pub(crate) async fn me(&self, user: UserId) -> Result<Me, Error> {
        let profile = self.workspace.profile(user).await?;
        let now = self.now().to_zoned(profile.time_zone.clone());
        Ok(Me { profile, now })
    }

    /// Every task that belongs to the person: their checklist items
    /// (R-MW-1) unioned with assigned issues, comments and reviews.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub(crate) async fn gather(&self, user: UserId, handle: &str) -> Result<Vec<Task>, Error> {
        let files = self.workspace.task_files(user).await?;
        let mut tasks: Vec<Task> = files
            .iter()
            .flat_map(|file| {
                projection::project_file(&file.context, &file.text)
                    .into_iter()
                    .filter(|t| projection::belongs_to(t, handle, file.context.owner.as_deref()))
            })
            .collect();
        tasks.extend(self.workspace.assigned(user).await?);
        Ok(tasks)
    }

    /// Reads a file, edits it and writes it back, retrying when someone
    /// else saved in between. Returns the file as written.
    pub(crate) async fn edit<T>(
        &self,
        user: UserId,
        doc: DocId,
        change: impl Fn(&Document) -> Result<(String, T), Error> + Send,
    ) -> Result<(Document, T), Error>
    where
        T: Send,
    {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let current = self.workspace.read(user, doc).await?;
            let (text, out) = change(&current)?;
            if text == current.text {
                return Ok((current, out));
            }
            match self
                .workspace
                .write(user, doc, text.clone(), current.version)
                .await
            {
                Ok(version) => {
                    self.touch(user);
                    let written = Document {
                        context: current.context,
                        text,
                        version,
                        modified: self.now(),
                    };
                    return Ok((written, out));
                }
                Err(err) if err.kind() == ErrorKind::Conflict && attempt < EDIT_ATTEMPTS => {}
                Err(err) => return Err(err),
            }
        }
    }

    fn task_view(doc: &Document, block: BlockId, today: Date) -> Result<TaskView, Error> {
        find_task(doc, block)
            .map(|task| TaskView::standalone(&task, today))
            .ok_or_else(|| not_found("no such task"))
    }

    /// The person's My Work view (R-MW-2).
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn my_work(&self, user: UserId, _input: NoInput) -> Result<MyWorkView, Error> {
        let me = self.me(user).await?;
        let tasks = self.gather(user, &me.profile.handle).await?;
        let view = projection::build(tasks, me.today());
        Ok(MyWorkView::new(&view, me.today()))
    }

    /// Parses a quick-add line without saving it, for live highlighting.
    ///
    /// # Errors
    ///
    /// Fails when the profile cannot be read.
    pub async fn parse(&self, user: UserId, input: ParseInput) -> Result<ParseView, Error> {
        let me = self.me(user).await?;
        let parsed = quickadd::parse(&input.text, me.now.datetime());
        Ok(ParseView {
            title: parsed.title,
            fields: FieldsView::new(&parsed.fields),
            list: parsed.list,
            assignees: parsed.assignees,
            highlights: parsed
                .tokens
                .iter()
                .map(|t| HighlightView {
                    start: utf16_offset(&input.text, t.span.start),
                    end: utf16_offset(&input.text, t.span.end),
                    kind: t.kind.into(),
                })
                .collect(),
        })
    }

    /// The person's Inbox, created on first use.
    pub(crate) async fn inbox(&self, user: UserId, handle: &str) -> Result<DocId, Error> {
        self.list_named(user, handle, None).await
    }

    /// The list called `title` (or the Inbox), created when missing.
    async fn list_named(
        &self,
        user: UserId,
        handle: &str,
        title: Option<&str>,
    ) -> Result<DocId, Error> {
        let files = self.workspace.task_files(user).await?;
        let is_target = |f: &&Document| {
            title.map_or_else(
                || f.context.origin == Origin::Inbox && f.context.owner.as_deref() == Some(handle),
                |title| {
                    f.context.origin == Origin::List && list_title(f).eq_ignore_ascii_case(title)
                },
            )
        };
        if let Some(file) = files.iter().find(is_target) {
            return Ok(file.context.doc);
        }
        let title = title.unwrap_or("Inbox");
        let path = if title == "Inbox" {
            lists::inbox_path(handle)
        } else {
            lists::list_path(handle, title)
        };
        let text = lists::new_list(DocId::generate(), title, handle);
        let created = self.workspace.create(user, path, text).await;
        self.touch(user);
        created
    }

    /// Adds a task from a quick-add line (R-MW-3).
    ///
    /// # Errors
    ///
    /// `invalid_input` for an empty title, `not_found` for an unknown
    /// parent.
    pub async fn create(&self, user: UserId, input: CreateInput) -> Result<TaskView, Error> {
        let me = self.me(user).await?;
        let parsed = quickadd::parse(&input.text, me.now.datetime());
        if parsed.title.trim().is_empty() {
            return Err(invalid("the task needs a title"));
        }
        let title = with_mentions(&parsed.title, &parsed.assignees);
        let (doc, parent) = if let Some(parent) = &input.parent {
            let (doc, block) = parse_task_id(parent)?;
            (doc, Some(block))
        } else {
            let list = parsed.list.as_deref().or(input.list.as_deref());
            let doc = match list {
                Some(name) if !name.eq_ignore_ascii_case("inbox") => {
                    self.list_named(user, &me.profile.handle, Some(name))
                        .await?
                }
                _ => self.inbox(user, &me.profile.handle).await?,
            };
            (doc, None)
        };
        let block = BlockId::generate();
        let (written, ()) = self
            .edit(user, doc, |file| {
                checklist::append(&file.text, parent, &title, &parsed.fields, block)
                    .map(|text| (text, ()))
                    .map_err(edit_error)
            })
            .await?;
        Self::task_view(&written, block, me.today())
    }

    /// Completes a task. A repeating task moves to its next date and
    /// stays open (R-MW-4).
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown task.
    pub async fn complete(
        &self,
        user: UserId,
        input: CompleteInput,
    ) -> Result<CompleteView, Error> {
        let me = self.me(user).await?;
        let (doc, block) = parse_task_id(&input.task)?;
        let now = me.now.clone();
        let (written, completion) = self
            .edit(user, doc, |file| {
                checklist::complete(&file.text, block, &now, input.cascade).map_err(edit_error)
            })
            .await?;
        let next = match completion {
            Completion::Done => None,
            Completion::Rescheduled { next } => Some(next.to_string()),
        };
        Ok(CompleteView {
            task: Self::task_view(&written, block, me.today())?,
            next,
        })
    }

    /// Reopens a completed task.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown task.
    pub async fn reopen(&self, user: UserId, input: TaskRef) -> Result<TaskView, Error> {
        let me = self.me(user).await?;
        let (doc, block) = parse_task_id(&input.task)?;
        let at = me.now.timestamp();
        let (written, ()) = self
            .edit(user, doc, |file| {
                checklist::set_checked(&file.text, block, false, at)
                    .map(|text| (text, ()))
                    .map_err(edit_error)
            })
            .await?;
        Self::task_view(&written, block, me.today())
    }

    /// Moves a task to another bucket: drag to reschedule (R-MW-2).
    ///
    /// # Errors
    ///
    /// `invalid_input` for an unknown target, `not_found` for an unknown
    /// task.
    pub async fn move_to(&self, user: UserId, input: MoveInput) -> Result<TaskView, Error> {
        let me = self.me(user).await?;
        let (doc, block) = parse_task_id(&input.task)?;
        let target = parse_target(&input.to)?;
        let today = me.today();
        let (written, ()) = self
            .edit(user, doc, |file| {
                let list = Checklist::parse(&file.text);
                let item = list.find(block).ok_or_else(|| not_found("no such task"))?;
                let fields = projection::reschedule(&item.fields, target, today);
                checklist::update(&file.text, block, &item.title, &fields)
                    .map(|text| (text, ()))
                    .map_err(edit_error)
            })
            .await?;
        Self::task_view(&written, block, today)
    }

    /// Edits a task's title or fields.
    ///
    /// # Errors
    ///
    /// `invalid_input` for a value that does not parse, `not_found` for
    /// an unknown task.
    pub async fn update(&self, user: UserId, input: UpdateInput) -> Result<TaskView, Error> {
        let me = self.me(user).await?;
        let (doc, block) = parse_task_id(&input.task)?;
        let changes = [
            ("when", &input.when),
            ("start", &input.start),
            ("due", &input.due),
            ("rrule", &input.rrule),
            ("recur_from", &input.recur_from),
            ("remind", &input.remind),
            ("p", &input.priority),
            ("labels", &input.labels),
            ("block", &input.block),
        ];
        if let Some(title) = &input.title
            && title.trim().is_empty()
        {
            return Err(invalid("the task needs a title"));
        }
        let (written, ()) = self
            .edit(user, doc, |file| {
                let list = Checklist::parse(&file.text);
                let item = list.find(block).ok_or_else(|| not_found("no such task"))?;
                let mut attrs = Attrs::new();
                item.fields.write_attrs(&mut attrs);
                for (key, value) in &changes {
                    if let Some(value) = value {
                        attrs.set(key, value.trim().to_owned());
                    }
                }
                let fields =
                    TaskFields::from_attrs(&attrs).map_err(|err| invalid(err.to_string()))?;
                let title = input
                    .title
                    .as_deref()
                    .map_or(item.title.as_str(), str::trim);
                checklist::update(&file.text, block, title, &fields)
                    .map(|text| (text, ()))
                    .map_err(edit_error)
            })
            .await?;
        Self::task_view(&written, block, me.today())
    }

    /// Removes a task and its subtasks from the file.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown task.
    pub async fn delete(&self, user: UserId, input: TaskRef) -> Result<DeletedView, Error> {
        let (doc, block) = parse_task_id(&input.task)?;
        let _ = self
            .edit(user, doc, |file| {
                checklist::remove(&file.text, block)
                    .map(|text| (text, ()))
                    .map_err(edit_error)
            })
            .await?;
        Ok(DeletedView { id: input.task })
    }

    fn list_view(file: &Document) -> ListView {
        let meta = lists::read_meta(&file.text);
        let open = projection::project_file(&file.context, &file.text)
            .iter()
            .filter(|t| !t.done)
            .count();
        ListView {
            doc: file.context.doc,
            title: list_title(file),
            inbox: file.context.origin == Origin::Inbox,
            owner: meta.owner.or_else(|| file.context.owner.clone()),
            members: meta.members,
            open,
        }
    }

    /// The person's lists and the lists shared with them (R-MW-8).
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn lists(&self, user: UserId, _input: NoInput) -> Result<ListsView, Error> {
        let files = self.workspace.task_files(user).await?;
        let mut lists: Vec<ListView> = files
            .iter()
            .filter(|f| is_list(f))
            .map(Self::list_view)
            .collect();
        lists.sort_by(|a, b| {
            b.inbox
                .cmp(&a.inbox)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        Ok(ListsView { lists })
    }

    /// Creates a list.
    ///
    /// # Errors
    ///
    /// `invalid_input` for an empty title, `conflict` when the person
    /// already has a list with that title.
    pub async fn create_list(
        &self,
        user: UserId,
        input: CreateListInput,
    ) -> Result<ListView, Error> {
        let me = self.me(user).await?;
        let title = input.title.trim();
        if title.is_empty() || title.eq_ignore_ascii_case("inbox") {
            return Err(invalid("choose another title"));
        }
        let files = self.workspace.task_files(user).await?;
        if files
            .iter()
            .any(|f| f.context.origin == Origin::List && list_title(f).eq_ignore_ascii_case(title))
        {
            return Err(Error::new(
                ErrorKind::Conflict,
                "a list with this title exists",
            ));
        }
        let doc = self
            .list_named(user, &me.profile.handle, Some(title))
            .await?;
        let file = self.workspace.read(user, doc).await?;
        Ok(Self::list_view(&file))
    }

    async fn set_member(
        &self,
        user: UserId,
        input: MemberInput,
        member: bool,
    ) -> Result<ListView, Error> {
        let me = self.me(user).await?;
        let handle = input.member.trim().trim_start_matches('@').to_owned();
        if handle.is_empty() {
            return Err(invalid("name a person"));
        }
        let file = self.workspace.read(user, input.list_id).await?;
        let meta = lists::read_meta(&file.text);
        if !is_list(&file) || meta.owner.as_deref() != Some(me.profile.handle.as_str()) {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "only the list's owner can change its members",
            ));
        }
        self.workspace
            .set_member(user, input.list_id, handle.clone(), member)
            .await?;
        let (written, ()) = self
            .edit(user, input.list_id, |file| {
                let text = if member {
                    lists::share(&file.text, &handle)
                } else {
                    lists::unshare(&file.text, &handle)
                };
                Ok((text.unwrap_or_else(|| file.text.clone()), ()))
            })
            .await?;
        Ok(Self::list_view(&written))
    }

    /// Shares a list with someone.
    ///
    /// # Errors
    ///
    /// `permission_denied` unless the caller owns the list.
    pub async fn share_list(&self, user: UserId, input: MemberInput) -> Result<ListView, Error> {
        self.set_member(user, input, true).await
    }

    /// Stops sharing a list with someone.
    ///
    /// # Errors
    ///
    /// `permission_denied` unless the caller owns the list.
    pub async fn unshare_list(&self, user: UserId, input: MemberInput) -> Result<ListView, Error> {
        self.set_member(user, input, false).await
    }

    /// Imports an export from another app into lists (R-MW-7). Running
    /// the same import again adds only what is new.
    ///
    /// # Errors
    ///
    /// `invalid_input` when the export cannot be read.
    pub async fn import(&self, user: UserId, input: ImportInput) -> Result<ImportView, Error> {
        let me = self.me(user).await?;
        let parsed = match input.from {
            ImportFrom::GoogleTasks => import::google_tasks(&input.content),
            ImportFrom::Todoist => import::todoist_csv(
                input.project.as_deref().unwrap_or("Todoist"),
                &input.content,
                me.now.datetime(),
            ),
            ImportFrom::Things => import::things_json(&input.content, me.today()),
        }
        .map_err(|err| invalid(err.to_string()))?;
        let mut out = Vec::new();
        for list in parsed {
            let files = self.workspace.task_files(user).await?;
            let existing = files.iter().find(|f| {
                f.context.origin == Origin::List
                    && f.context.owner.as_deref() == Some(me.profile.handle.as_str())
                    && list_title(f).eq_ignore_ascii_case(&list.title)
            });
            let (doc, added) = if let Some(file) = existing {
                let (_, added) = self
                    .edit(user, file.context.doc, |file| {
                        Ok(import::append_new(&file.text, &list)
                            .unwrap_or_else(|| (file.text.clone(), 0)))
                    })
                    .await?;
                (file.context.doc, added)
            } else {
                let text = import::render_list(&list, DocId::generate(), &me.profile.handle);
                let id = self
                    .workspace
                    .create(
                        user,
                        lists::list_path(&me.profile.handle, &list.title),
                        text,
                    )
                    .await?;
                (id, list.count())
            };
            out.push(ImportedList {
                doc,
                title: list.title,
                added,
            });
        }
        Ok(ImportView { lists: out })
    }

    /// Issues a new calendar feed URL, revoking the old one (R-MW-6).
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn rotate_feed(&self, user: UserId, _input: NoInput) -> Result<SecretView, Error> {
        let (token, hash) = feed::issue_token().map_err(|err| {
            Error::new(ErrorKind::Internal, "could not issue a token").with_source(err)
        })?;
        self.workspace
            .set_secret(user, SecretKind::Feed, Some(hash))
            .await?;
        Ok(SecretView {
            value: format!(
                "{}/feeds/tasks/{}.ics",
                self.config.base_url.trim_end_matches('/'),
                token.expose()
            ),
        })
    }

    /// Where the person's tasks connect: the CalDAV URL and whether
    /// e-mail and Slack capture are available (D298). The feed URL and the
    /// capture address are shown only when issued, since only hashes are kept.
    ///
    /// # Errors
    ///
    /// Fails when the profile cannot be read.
    pub async fn connections(
        &self,
        user: UserId,
        _input: NoInput,
    ) -> Result<ConnectionsView, Error> {
        let profile = self.workspace.profile(user).await?;
        let slack = self.config.slack_signing_secret.is_some();
        let slack_linked = slack && self.workspace.slack_linked(profile.handle.clone()).await?;
        Ok(ConnectionsView {
            caldav_url: format!(
                "{}{}",
                self.config.base_url.trim_end_matches('/'),
                caldav::href(&caldav::Resource::Home {
                    user: profile.handle
                })
            ),
            email: !self.config.mail_domain.trim().is_empty(),
            slack,
            slack_linked,
        })
    }

    /// Turns the calendar feed off.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn revoke_feed(&self, user: UserId, _input: NoInput) -> Result<Done, Error> {
        self.workspace
            .set_secret(user, SecretKind::Feed, None)
            .await?;
        Ok(Done {})
    }

    /// Issues a new capture e-mail address, revoking the old one.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn rotate_capture(&self, user: UserId, _input: NoInput) -> Result<SecretView, Error> {
        let (token, hash) = feed::issue_token().map_err(|err| {
            Error::new(ErrorKind::Internal, "could not issue a token").with_source(err)
        })?;
        self.workspace
            .set_secret(user, SecretKind::Capture, Some(hash))
            .await?;
        Ok(SecretView {
            value: capture::address(&token, &self.config.mail_domain),
        })
    }

    /// Turns capture by e-mail off.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn revoke_capture(&self, user: UserId, _input: NoInput) -> Result<Done, Error> {
        self.workspace
            .set_secret(user, SecretKind::Capture, None)
            .await?;
        Ok(Done {})
    }

    /// Links the Slack user named by a code from the "Save as task"
    /// shortcut to the caller (#163, D177).
    ///
    /// # Errors
    ///
    /// Slack capture not set up, or a forged or expired code.
    pub async fn link_slack(&self, user: UserId, input: SlackLinkInput) -> Result<Done, Error> {
        let Some(secret) = &self.config.slack_signing_secret else {
            return Err(Error::new(
                ErrorKind::Unavailable,
                "Slack capture is not set up on this server",
            ));
        };
        let (team, slack_user) = capture::read_slack_link_code(secret, &input.code, self.now())
            .map_err(|_| {
                Error::new(
                    ErrorKind::InvalidInput,
                    "this Slack link is not valid or has expired; use Save as task in Slack again",
                )
            })?;
        self.workspace.link_slack(user, team, slack_user).await?;
        tracing::info!("Slack account linked for capture");
        Ok(Done {})
    }

    /// Removes the caller's Slack links.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn unlink_slack(&self, user: UserId, _input: NoInput) -> Result<Done, Error> {
        let _ = self.workspace.unlink_slack(user).await?;
        Ok(Done {})
    }

    /// Today's digest (R-MW-5); the notifications service sends it.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn digest(&self, user: UserId, _input: NoInput) -> Result<DigestView, Error> {
        let me = self.me(user).await?;
        let tasks = self.gather(user, &me.profile.handle).await?;
        let view = projection::build(tasks, me.today());
        Ok(DigestView {
            markdown: remind::digest(&view, me.today()),
        })
    }

    /// Reminders for the person that fire in `(after, until]`. The
    /// notifications service calls this on a timer.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn reminders_due(
        &self,
        user: UserId,
        after: Timestamp,
        until: Timestamp,
    ) -> Result<Vec<remind::DueReminder>, Error> {
        let profile = self.workspace.profile(user).await?;
        let settings = profile.preferences.reminders(profile.time_zone.clone());
        let tasks = self.gather(user, &profile.handle).await?;
        Ok(remind::due_between(&tasks, after, until, &settings))
    }

    /// The person's My Work settings (D178).
    ///
    /// # Errors
    ///
    /// Fails when the profile cannot be read.
    pub async fn settings(
        &self,
        user: UserId,
        _input: NoInput,
    ) -> Result<MyWorkSettingsView, Error> {
        let profile = self.workspace.profile(user).await?;
        Ok(MyWorkSettingsView::new(
            &profile.preferences,
            &profile.time_zone,
        ))
    }

    /// Replaces the person's My Work settings (D178).
    ///
    /// # Errors
    ///
    /// `invalid` for an unknown zone or a malformed time.
    pub async fn set_settings(
        &self,
        user: UserId,
        input: MyWorkSettingsInput,
    ) -> Result<MyWorkSettingsView, Error> {
        let preferences = Preferences::parse(&input)?;
        self.workspace.save_preferences(user, preferences).await?;
        self.touch(user);
        self.settings(user, NoInput {}).await
    }

    /// Takes a mention out of the person's My Work (D179).
    ///
    /// # Errors
    ///
    /// `invalid` for a malformed comment ID.
    pub async fn dismiss_mention(&self, user: UserId, input: MentionRef) -> Result<Done, Error> {
        let comment: CommentId = input
            .comment
            .parse()
            .map_err(|_| invalid("not a comment ID"))?;
        self.workspace.dismiss_mention(user, comment).await?;
        Ok(Done {})
    }

    /// Tidies a file after the editor saves it: gives new checklist items
    /// block IDs and moves repeating items ticked in the editor to their
    /// next date. The store's save hook calls this.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn settle(&self, user: UserId, doc: DocId) -> Result<(), Error> {
        // The editor saved: whatever it wrote may hold new reminders.
        self.touch(user);
        let me = self.me(user).await?;
        let now = me.now;
        let _ = self
            .edit(user, doc, |file| {
                let ided = checklist::ensure_block_ids(&file.text);
                let base = ided.as_deref().unwrap_or(&file.text);
                let settled = checklist::settle(base, &now);
                Ok((settled.or(ided).unwrap_or_else(|| file.text.clone()), ()))
            })
            .await?;
        Ok(())
    }
}

/// Wraps a service method as an operation handler.
macro_rules! handler {
    ($service:expr, $method:ident) => {{
        let service = Arc::clone(&$service);
        move |ctx: RequestContext, input| {
            let service = Arc::clone(&service);
            async move {
                let user = ctx.principal.require_user()?;
                service.$method(user, input).await
            }
        }
    }};
}

impl Service for TasksService {
    fn name(&self) -> &'static str {
        "tasks"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let mut operations = task_operations(&self);
        operations.extend(list_operations(&self));
        operations.extend(settings_operations(&self));
        operations.extend(google_operations(&self));
        operations
    }
}

fn task_operations(s: &Arc<TasksService>) -> Vec<Operation> {
    use HttpMethod::{Delete, Patch, Post};
    vec![
        Operation::query(
            "tasks.my_work",
            "/v1/my-work",
            Scope::Read,
            handler!(s, my_work),
        )
        .summary("Lists My Work: Inbox, Today, Evening, Upcoming, Anytime, Someday, Done"),
        Operation::query(
            "tasks.parse",
            "/v1/tasks/parse",
            Scope::Read,
            handler!(s, parse),
        )
        .summary("Parses a quick-add line for highlighting"),
        Operation::mutation(
            "tasks.create",
            Post,
            "/v1/tasks",
            Scope::WriteDirect,
            handler!(s, create),
        )
        .summary("Adds a task from a quick-add line"),
        Operation::mutation(
            "tasks.complete",
            Post,
            "/v1/tasks/{task}/complete",
            Scope::WriteDirect,
            handler!(s, complete),
        )
        .summary("Completes a task; a repeating task moves to its next date"),
        Operation::mutation(
            "tasks.reopen",
            Post,
            "/v1/tasks/{task}/reopen",
            Scope::WriteDirect,
            handler!(s, reopen),
        )
        .summary("Reopens a completed task"),
        Operation::mutation(
            "tasks.move",
            Post,
            "/v1/tasks/{task}/move",
            Scope::WriteDirect,
            handler!(s, move_to),
        )
        .summary("Moves a task to another bucket or day"),
        Operation::mutation(
            "tasks.update",
            Patch,
            "/v1/tasks/{task}",
            Scope::WriteDirect,
            handler!(s, update),
        )
        .summary("Edits a task's title or fields"),
        Operation::mutation(
            "tasks.delete",
            Delete,
            "/v1/tasks/{task}",
            Scope::WriteDirect,
            handler!(s, delete),
        )
        .summary("Removes a task and its subtasks"),
        Operation::query(
            "tasks.digest",
            "/v1/my-work/digest",
            Scope::Read,
            handler!(s, digest),
        )
        .summary("Today's digest as Vellum Markdown"),
    ]
}

fn list_operations(s: &Arc<TasksService>) -> Vec<Operation> {
    use HttpMethod::{Delete, Post};
    vec![
        Operation::query(
            "tasks.lists",
            "/v1/task-lists",
            Scope::Read,
            handler!(s, lists),
        )
        .summary("Lists the person's task lists and lists shared with them"),
        Operation::mutation(
            "tasks.create_list",
            Post,
            "/v1/task-lists",
            Scope::WriteDirect,
            handler!(s, create_list),
        )
        .summary("Creates a task list"),
        Operation::mutation(
            "tasks.share_list",
            Post,
            "/v1/task-lists/{list_id}/members",
            Scope::WriteDirect,
            handler!(s, share_list),
        )
        .summary("Shares a list with someone"),
        Operation::mutation(
            "tasks.unshare_list",
            Delete,
            "/v1/task-lists/{list_id}/members/{member}",
            Scope::WriteDirect,
            handler!(s, unshare_list),
        )
        .summary("Stops sharing a list with someone"),
        Operation::mutation(
            "tasks.import",
            Post,
            "/v1/tasks/import",
            Scope::WriteDirect,
            handler!(s, import),
        )
        .summary("Imports tasks from Google Tasks, Todoist or Things")
        .budget(Duration::from_mins(1)),
        Operation::query(
            "tasks.connections",
            "/v1/my-work/connections",
            Scope::Read,
            handler!(s, connections),
        )
        .summary("Where your tasks connect: CalDAV, e-mail and Slack capture"),
        Operation::mutation(
            "tasks.rotate_feed",
            Post,
            "/v1/my-work/feed",
            Scope::WriteDirect,
            handler!(s, rotate_feed),
        )
        .summary("Issues a new calendar feed URL and revokes the old one"),
        Operation::mutation(
            "tasks.revoke_feed",
            Delete,
            "/v1/my-work/feed",
            Scope::WriteDirect,
            handler!(s, revoke_feed),
        )
        .summary("Turns the calendar feed off"),
        Operation::mutation(
            "tasks.rotate_capture",
            Post,
            "/v1/my-work/capture-address",
            Scope::WriteDirect,
            handler!(s, rotate_capture),
        )
        .summary("Issues a new capture e-mail address and revokes the old one"),
        Operation::mutation(
            "tasks.revoke_capture",
            Delete,
            "/v1/my-work/capture-address",
            Scope::WriteDirect,
            handler!(s, revoke_capture),
        )
        .summary("Turns capture by e-mail off"),
        Operation::mutation(
            "tasks.link_slack",
            Post,
            "/v1/my-work/slack-link",
            Scope::WriteDirect,
            handler!(s, link_slack),
        )
        .summary("Links your Slack account for Save as task, with the code Slack showed you"),
        Operation::mutation(
            "tasks.unlink_slack",
            Delete,
            "/v1/my-work/slack-link",
            Scope::WriteDirect,
            handler!(s, unlink_slack),
        )
        .summary("Unlinks your Slack account"),
    ]
}

fn settings_operations(s: &Arc<TasksService>) -> Vec<Operation> {
    vec![
        Operation::query(
            "tasks.settings",
            "/v1/my-work/settings",
            Scope::Read,
            handler!(s, settings),
        )
        .summary("Your My Work settings: time zone, start of day and digest time"),
        Operation::mutation(
            "tasks.settings_update",
            HttpMethod::Put,
            "/v1/my-work/settings",
            Scope::WriteDirect,
            handler!(s, set_settings),
        )
        .summary("Replaces your My Work settings"),
        Operation::mutation(
            "tasks.mention_dismiss",
            HttpMethod::Post,
            "/v1/my-work/mentions/{comment}/dismiss",
            Scope::WriteDirect,
            handler!(s, dismiss_mention),
        )
        .summary("Takes a mention out of My Work without replying"),
    ]
}

fn google_operations(s: &Arc<TasksService>) -> Vec<Operation> {
    use HttpMethod::{Delete, Post};
    vec![
        Operation::query(
            "tasks.google_status",
            "/v1/my-work/google",
            Scope::Read,
            handler!(s, google_status),
        )
        .summary("Whether Google Calendar time-blocking is set up and connected"),
        Operation::mutation(
            "tasks.google_connect",
            Post,
            "/v1/my-work/google/connect",
            Scope::WriteDirect,
            handler!(s, google_connect),
        )
        .summary("Starts connecting Google Calendar; returns Google's consent page"),
        Operation::mutation(
            "tasks.google_sync",
            Post,
            "/v1/my-work/google/sync",
            Scope::WriteDirect,
            handler!(s, google_sync),
        )
        .summary("Syncs time blocks with Google Calendar now")
        .budget(Duration::from_mins(1)),
        Operation::mutation(
            "tasks.google_disconnect",
            Delete,
            "/v1/my-work/google",
            Scope::WriteDirect,
            handler!(s, google_disconnect),
        )
        .summary("Disconnects Google Calendar"),
    ]
}
