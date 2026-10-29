// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The My Work projection: tasks from checklist files, unioned with the
//! issues, comments and review requests assigned to a person, sorted into
//! buckets (R-MW-1, D19).
//!
//! Checklist items are projected into task rows keyed by document and
//! block ID; issues, comments and reviews are not copied, they are joined
//! in when the view is built, so nothing can drift.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use jiff::ToSpan as _;
use jiff::civil::{Date, Weekday};
use vellum_kernel::DocId;

use crate::checklist::Checklist;
use crate::model::{Due, IssueCategory, Origin, Priority, Source, Task, TaskFields, When};
use crate::telemetry;

/// The file a checklist lives in, as the index knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContext {
    /// The document or list file.
    pub doc: DocId,
    /// What kind of file it is.
    pub origin: Origin,
    /// The handle of the list's owner, for files under `tasks/<user>/`.
    pub owner: Option<String>,
    /// The list's title, for files under `tasks/`.
    pub list: Option<String>,
}

/// Projects every checklist item with a block ID in a file into a task.
/// Items without a block ID are skipped until the editor (or
/// [`crate::checklist::ensure_block_ids`]) gives them one.
#[must_use]
pub fn project_file(context: &FileContext, source: &str) -> Vec<Task> {
    let _span = tracing::debug_span!("tasks.project_file", doc = %context.doc).entered();
    let list = Checklist::parse(source);
    let tasks: Vec<Task> = list
        .items
        .iter()
        .filter_map(|item| {
            let block = item.block?;
            let parent = item
                .parent
                .and_then(|p| list.items.get(p))
                .and_then(|p| p.block);
            Some(Task {
                source: Source::Checklist {
                    doc: context.doc,
                    block,
                    origin: context.origin,
                },
                title: item.title.clone(),
                done: item.checked,
                fields: item.fields.clone(),
                assignees: item.mentions.clone(),
                parent,
                list: context.list.clone(),
            })
        })
        .collect();
    telemetry::record_projected(tasks.len());
    tasks
}

/// Whether a projected task belongs in `me`'s My Work.
///
/// Items in a list belong to the people they mention, or to the list's owner when they
/// mention nobody; items in documents belong to the people they mention.
/// Issues, comments and reviews are already selected for `me` by the
/// union query.
#[must_use]
pub fn belongs_to(task: &Task, me: &str, owner: Option<&str>) -> bool {
    match &task.source {
        Source::Checklist { origin, .. } => {
            let mentioned = task.assignees.iter().any(|a| a.eq_ignore_ascii_case(me));
            match origin {
                Origin::Doc => mentioned,
                Origin::Inbox | Origin::List => {
                    mentioned
                        || (task.assignees.is_empty()
                            && owner.is_some_and(|o| o.eq_ignore_ascii_case(me)))
                }
            }
        }
        Source::Issue { .. }
        | Source::Comment { .. }
        | Source::Mention { .. }
        | Source::Review { .. } => true,
    }
}

/// A My Work bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bucket {
    /// Not yet sorted.
    Inbox,
    /// To do today, including anything overdue.
    Today,
    /// Today, after work.
    Evening,
    /// Starts or is due on a later day.
    Upcoming,
    /// Ready whenever, with no date.
    Anytime,
    /// Parked.
    Someday,
    /// Completed.
    Done,
}

/// Puts a task in its bucket for the day `today` (the owner's local date).
#[must_use]
pub fn bucket(task: &Task, today: Date) -> Bucket {
    if task.done {
        return Bucket::Done;
    }
    let fields = &task.fields;
    let due = fields.due.map(Due::date);
    let dates = [fields.start, due];
    let today_or_earlier = dates.iter().flatten().any(|d| *d <= today);
    if today_or_earlier {
        return if fields.when == Some(When::Evening) {
            Bucket::Evening
        } else {
            Bucket::Today
        };
    }
    if dates.iter().flatten().next().is_some() {
        return Bucket::Upcoming;
    }
    match fields.when {
        Some(When::Evening) => return Bucket::Evening,
        Some(When::Someday) => return Bucket::Someday,
        Some(When::Anytime) => return Bucket::Anytime,
        None => {}
    }
    match &task.source {
        Source::Checklist { origin, .. } => match origin {
            Origin::Inbox => Bucket::Inbox,
            Origin::List | Origin::Doc => Bucket::Anytime,
        },
        Source::Comment { .. } | Source::Mention { .. } => Bucket::Inbox,
        Source::Review { .. } => Bucket::Today,
        Source::Issue { category, .. } => match category {
            IssueCategory::Started => Bucket::Today,
            IssueCategory::Triage | IssueCategory::Backlog | IssueCategory::Unstarted => {
                Bucket::Anytime
            }
            IssueCategory::Completed | IssueCategory::Canceled | IssueCategory::Duplicate => {
                Bucket::Done
            }
        },
    }
}

/// A heading inside Upcoming: one per day for the next seven days, then
/// one per week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UpcomingGroup {
    /// A single day within the next seven.
    Day(Date),
    /// The week starting on this Monday.
    Week(Date),
}

/// The day an upcoming task is listed under: its start date when it has
/// one, otherwise its deadline.
#[must_use]
pub fn upcoming_date(fields: &TaskFields) -> Option<Date> {
    fields.start.or_else(|| fields.due.map(Due::date))
}

/// The Upcoming heading for a day after `today`.
#[must_use]
pub fn upcoming_group(date: Date, today: Date) -> UpcomingGroup {
    let horizon = today.checked_add(7.days()).unwrap_or(today);
    if date <= horizon {
        return UpcomingGroup::Day(date);
    }
    let back = i64::from(date.weekday().to_monday_zero_offset());
    UpcomingGroup::Week(date.checked_sub(back.days()).unwrap_or(date))
}

/// One row of the view, with its subtasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The task.
    pub task: Task,
    /// Whether its deadline has passed.
    pub overdue: bool,
    /// Its subtasks in the same file, in file order.
    pub children: Vec<Self>,
}

/// My Work for one person on one day.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MyWork {
    /// Inbox.
    pub inbox: Vec<Entry>,
    /// Today.
    pub today: Vec<Entry>,
    /// This evening.
    pub evening: Vec<Entry>,
    /// Upcoming, grouped by day and then by week.
    pub upcoming: Vec<(UpcomingGroup, Vec<Entry>)>,
    /// Anytime.
    pub anytime: Vec<Entry>,
    /// Someday.
    pub someday: Vec<Entry>,
    /// Done, most recently completed first.
    pub done: Vec<Entry>,
}

impl MyWork {
    /// How many open tasks the view shows (subtasks not counted).
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.inbox.len()
            + self.today.len()
            + self.evening.len()
            + self.upcoming.iter().map(|(_, e)| e.len()).sum::<usize>()
            + self.anytime.len()
            + self.someday.len()
    }
}

fn order(a: &Entry, b: &Entry) -> Ordering {
    let due_key = |e: &Entry| e.task.fields.due;
    b.overdue
        .cmp(&a.overdue)
        .then_with(|| {
            let pa = a.task.fields.priority.map_or(5, Priority::level);
            let pb = b.task.fields.priority.map_or(5, Priority::level);
            pa.cmp(&pb)
        })
        .then_with(|| match (due_key(a), due_key(b)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
}

/// Builds a view row with its subtasks, taking them out of `children`.
fn entry(task: Task, today: Date, children: &mut BTreeMap<String, Vec<Task>>) -> Entry {
    let own = children.remove(&task.source.key()).unwrap_or_default();
    let kids = own
        .into_iter()
        .map(|child| entry(child, today, children))
        .collect();
    Entry {
        overdue: !task.done && task.fields.due.is_some_and(|d| d.date() < today),
        task,
        children: kids,
    }
}

/// Builds the view from the tasks that belong to the person.
///
/// `tasks` is the union of their checklist tasks and their assigned
/// issues, comments and reviews, in index order. A subtask whose parent
/// is also in the set is shown under it; otherwise it stands alone.
#[must_use]
pub fn build(tasks: Vec<Task>, today: Date) -> MyWork {
    let _span = tracing::debug_span!("tasks.build_view", tasks = tasks.len()).entered();
    let keys: Vec<String> = tasks.iter().map(|t| t.source.key()).collect();
    let parent_key = |task: &Task| -> Option<String> {
        let (Source::Checklist { doc, .. }, Some(parent)) = (&task.source, task.parent) else {
            return None;
        };
        Some(format!("{doc}.{parent}"))
    };
    let mut children: BTreeMap<String, Vec<Task>> = BTreeMap::new();
    let mut roots = Vec::new();
    for task in tasks {
        match parent_key(&task).filter(|k| keys.contains(k)) {
            Some(parent) => children.entry(parent).or_default().push(task),
            None => roots.push(task),
        }
    }
    let mut view = MyWork::default();
    let mut upcoming: BTreeMap<UpcomingGroup, Vec<Entry>> = BTreeMap::new();
    for task in roots {
        let which = bucket(&task, today);
        let item = entry(task, today, &mut children);
        match which {
            Bucket::Inbox => view.inbox.push(item),
            Bucket::Today => view.today.push(item),
            Bucket::Evening => view.evening.push(item),
            Bucket::Upcoming => {
                let date = upcoming_date(&item.task.fields).unwrap_or(today);
                let group = upcoming_group(date.max(today), today);
                upcoming.entry(group).or_default().push(item);
            }
            Bucket::Anytime => view.anytime.push(item),
            Bucket::Someday => view.someday.push(item),
            Bucket::Done => view.done.push(item),
        }
    }
    for list in [&mut view.today, &mut view.evening, &mut view.anytime] {
        list.sort_by(order);
    }
    view.upcoming = upcoming
        .into_iter()
        .map(|(group, mut entries)| {
            entries.sort_by(|a, b| {
                upcoming_date(&a.task.fields)
                    .cmp(&upcoming_date(&b.task.fields))
                    .then_with(|| order(a, b))
            });
            (group, entries)
        })
        .collect();
    view.done
        .sort_by_key(|e| std::cmp::Reverse(e.task.fields.done));
    view
}

/// Where a task is dragged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Back to the Inbox.
    Inbox,
    /// Today.
    Today,
    /// This evening.
    Evening,
    /// A later day.
    Day(Date),
    /// Anytime.
    Anytime,
    /// Someday.
    Someday,
}

/// The fields after dragging a task to another bucket ("drag to
/// reschedule", R-MW-1). Dragging moves the start date; the deadline
/// is a commitment and stays.
#[must_use]
pub fn reschedule(fields: &TaskFields, target: Target, today: Date) -> TaskFields {
    let mut fields = fields.clone();
    let (start, when) = match target {
        Target::Inbox => (None, None),
        Target::Today => (Some(today), None),
        Target::Evening => (Some(today), Some(When::Evening)),
        Target::Day(date) => (Some(date), None),
        Target::Anytime => (None, Some(When::Anytime)),
        Target::Someday => (None, Some(When::Someday)),
    };
    fields.start = start;
    fields.when = when;
    fields
}

/// The next day after `today` that falls on `weekday` (never today).
#[must_use]
pub fn next_weekday(today: Date, weekday: Weekday) -> Date {
    let ahead = (weekday.to_monday_zero_offset() - today.weekday().to_monday_zero_offset() + 6)
        .rem_euclid(7)
        + 1;
    today.checked_add(i64::from(ahead).days()).unwrap_or(today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Priority;
    use jiff::civil::date;
    use vellum_kernel::IssueId;

    fn doc() -> DocId {
        "01M639NFH15EBMG6C7NX1ZK4KA".parse().unwrap()
    }

    const LIST: &str = "- [ ] Inbox thing ^blk-00000001\n\
- [ ] Today thing {start=2026-11-02} ^blk-00000002\n\
- [ ] Overdue {due=2026-10-30 p=2} ^blk-00000003\n\
- [ ] Evening {when=evening start=2026-11-02} ^blk-00000004\n\
- [ ] Soon {due=2026-11-05} ^blk-00000005\n\
- [ ] Later {start=2026-11-20} ^blk-00000006\n\
- [ ] Parked {when=someday} ^blk-00000007\n\
- [ ] For Ann @ann ^blk-00000008\n\
- [x] Finished {done=2026-11-01T10:00:00Z} ^blk-00000009\n\
- [ ] Parent {start=2026-11-02} ^blk-0000000a\n  - [ ] Child ^blk-0000000b\n\
- [ ] No ID yet\n";

    fn context(origin: Origin) -> FileContext {
        FileContext {
            doc: doc(),
            origin,
            owner: Some("gourav".into()),
            list: Some("Inbox".into()),
        }
    }

    #[test]
    fn projects_and_buckets_a_list() {
        let tasks = project_file(&context(Origin::Inbox), LIST);
        assert_eq!(tasks.len(), 11);
        let mine: Vec<Task> = tasks
            .into_iter()
            .filter(|t| belongs_to(t, "gourav", Some("gourav")))
            .collect();
        assert_eq!(mine.len(), 10);
        let view = build(mine, date(2026, 11, 2));
        let titles = |entries: &[Entry]| -> Vec<String> {
            entries.iter().map(|e| e.task.title.clone()).collect()
        };
        assert_eq!(titles(&view.inbox), ["Inbox thing"]);
        assert_eq!(titles(&view.today), ["Overdue", "Today thing", "Parent"]);
        assert!(view.today[0].overdue);
        assert_eq!(titles(&view.today[2].children), ["Child"]);
        assert_eq!(titles(&view.evening), ["Evening"]);
        assert_eq!(view.upcoming.len(), 2);
        assert_eq!(view.upcoming[0].0, UpcomingGroup::Day(date(2026, 11, 5)));
        assert_eq!(view.upcoming[1].0, UpcomingGroup::Week(date(2026, 11, 16)));
        assert_eq!(titles(&view.someday), ["Parked"]);
        assert_eq!(titles(&view.done), ["Finished"]);
        assert_eq!(view.open_count(), 8);
    }

    #[test]
    fn documents_only_show_mentioned_items() {
        let tasks = project_file(&context(Origin::Doc), LIST);
        let mine: Vec<&Task> = tasks
            .iter()
            .filter(|t| belongs_to(t, "ann", None))
            .collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(bucket(mine[0], date(2026, 11, 2)), Bucket::Anytime);
    }

    #[test]
    fn issues_comments_and_reviews_join_the_view() {
        let issue = |category| Task {
            source: Source::Issue {
                issue: IssueId::generate(),
                key: "ENG-142".into(),
                category,
            },
            title: "Fix export".into(),
            done: category.is_closed(),
            fields: TaskFields::default(),
            assignees: vec![],
            parent: None,
            list: None,
        };
        let today = date(2026, 11, 2);
        assert_eq!(bucket(&issue(IssueCategory::Started), today), Bucket::Today);
        assert_eq!(
            bucket(&issue(IssueCategory::Backlog), today),
            Bucket::Anytime
        );
        assert_eq!(
            bucket(&issue(IssueCategory::Completed), today),
            Bucket::Done
        );
        let review = Task {
            source: Source::Review {
                reference: "doc:x".into(),
                given: false,
            },
            ..issue(IssueCategory::Started)
        };
        assert_eq!(bucket(&review, today), Bucket::Today);
        assert!(belongs_to(&review, "anyone", None));
    }

    #[test]
    fn rescheduling_moves_the_start_and_keeps_the_deadline() {
        let fields = TaskFields {
            due: Some(Due::Date(date(2026, 11, 9))),
            priority: Priority::new(1),
            ..TaskFields::default()
        };
        let today = date(2026, 11, 2);
        let moved = reschedule(&fields, Target::Evening, today);
        assert_eq!(moved.start, Some(today));
        assert_eq!(moved.when, Some(When::Evening));
        assert_eq!(moved.due, fields.due);
        let parked = reschedule(&moved, Target::Someday, today);
        assert_eq!((parked.start, parked.when), (None, Some(When::Someday)));
        assert_eq!(
            reschedule(&fields, Target::Day(date(2026, 11, 4)), today).start,
            Some(date(2026, 11, 4))
        );
    }

    #[test]
    fn next_weekday_is_never_today() {
        // 2026-11-02 is a Monday.
        assert_eq!(
            next_weekday(date(2026, 11, 2), Weekday::Monday),
            date(2026, 11, 9)
        );
        assert_eq!(
            next_weekday(date(2026, 11, 2), Weekday::Tuesday),
            date(2026, 11, 3)
        );
        assert_eq!(
            next_weekday(date(2026, 11, 2), Weekday::Sunday),
            date(2026, 11, 8)
        );
    }
}
