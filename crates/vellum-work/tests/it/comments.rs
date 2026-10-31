// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Comments, assignments from comments, and issue ↔ doc links on the
//! Northstar demo workspace.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "fixture helpers panic on bad test data"
)]

use std::collections::BTreeSet;

use time::OffsetDateTime;
use time::macros::datetime;
use vellum_kernel::{CommentId, DocId};
use vellum_work::comment::AuthoredBy;
use vellum_work::doclink::{self, IssueEmbed};
use vellum_work::{
    BlockRef, Comments, Ctx, IssueId, NewComment, NewIssue, Target, Team,
    TeamId, TeamKey, Thread, UserRef, WorkError, WorkEvent, WorkGraph,
};

const NOW: OffsetDateTime = datetime!(2026-11-02 09:00 UTC);

fn user(h: &str) -> UserRef {
    UserRef::parse(h).unwrap()
}

fn tk(s: &str) -> TeamKey {
    TeamKey::parse(s).unwrap()
}

fn as_user(h: &str) -> Ctx {
    Ctx {
        now: NOW,
        today: NOW.date(),
        actor: Some(user(h)),
    }
}

fn cid(n: u32) -> CommentId {
    format!("01JD000000000000000000{n:04}").parse().unwrap()
}

fn iid(n: u32) -> IssueId {
    format!("01JC000000000000000000{n:04}").parse().unwrap()
}

fn plan_doc() -> DocId {
    "01JB3K9Q7Z5N8M2X4V6T0R1Y3W".parse().unwrap()
}

const PLAN: &str = "---\nid: 01JB3K9Q7Z5N8M2X4V6T0R1Y3W\ntitle: Gateway plan\n\
---\n# Gateway plan ^blk-aaaaaaaa\n\n\
Rate limits drop bursts from the mobile app. ^blk-bbbbbbbb\n\n\
Retry storms are tracked in ENG-1. ^blk-cccccccc\n";

/// ENG (dev, priya; lead mara) and OPS, with ENG-1 filed by mara.
fn northstar() -> WorkGraph {
    let mut g = WorkGraph::new();
    let mut eng = Team::new(
        "01JC00000000000000000000E1".parse::<TeamId>().unwrap(),
        tk("ENG"),
        "Engineering",
    )
    .unwrap();
    eng.lead = Some(user("mara"));
    eng.members = vec![user("dev"), user("priya")];
    g.insert_team(eng).unwrap();
    let ops = Team::new(
        "01JC00000000000000000000E2".parse::<TeamId>().unwrap(),
        tk("OPS"),
        "Operations",
    )
    .unwrap();
    g.insert_team(ops).unwrap();
    let _ = g
        .create_issue(
            NewIssue::new(iid(1), tk("ENG"), "Retry storms"),
            &as_user("mara"),
        )
        .unwrap();
    g
}

fn added(
    changes: &vellum_work::CommentChanges,
) -> (Vec<String>, Option<String>) {
    match changes.events.first() {
        Some(WorkEvent::CommentAdded {
            recipients,
            assignee,
            ..
        }) => (
            recipients.iter().map(ToString::to_string).collect(),
            assignee.as_ref().map(ToString::to_string),
        ),
        other => panic!("expected CommentAdded, got {other:?}"),
    }
}

#[test]
fn issue_threads_notify_and_subscribe() {
    let mut g = northstar();
    let mut c = Comments::new();
    let target = g.comment_target("ENG-1").unwrap();
    let subscribers = g.issue(&iid(1)).unwrap().subscribers.clone();

    let draft =
        NewComment::new(cid(1), target, "@dev can you take a look? cc `@sam`");
    let changes = c.add(draft, &subscribers, &as_user("priya")).unwrap();
    let (recipients, _) = added(&changes);
    assert!(recipients.contains(&"dev".to_owned()), "{recipients:?}");
    assert!(
        !recipients.contains(&"priya".to_owned()),
        "the author is not notified"
    );
    assert!(
        !recipients.contains(&"sam".to_owned()),
        "code is not a mention"
    );

    let issue_changes = g.apply_comment(&changes, &as_user("priya")).unwrap();
    assert!(issue_changes.issues.contains(&iid(1)));
    let subs = &g.issue(&iid(1)).unwrap().subscribers;
    assert!(subs.contains(&user("priya")) && subs.contains(&user("dev")));

    // A reply goes to the thread; participants hear about it.
    let reply = NewComment::reply(cid(2), cid(1), "On it.");
    let changes = c.add(reply, &[], &as_user("dev")).unwrap();
    assert_eq!(added(&changes).0, ["priya"]);
    assert_eq!(c.thread(&cid(1)).unwrap().comments.len(), 2);
    assert_eq!(c.thread_of(&cid(2)).unwrap().id, cid(1));

    // Resolving, then replying, reopens the thread.
    let _ = c.resolve(&cid(1), true, &as_user("priya")).unwrap();
    assert!(c.thread(&cid(1)).unwrap().is_resolved());
    let changes = c
        .add(
            NewComment::reply(cid(3), cid(2), "Back again."),
            &[],
            &as_user("dev"),
        )
        .unwrap();
    assert!(!c.thread(&cid(1)).unwrap().is_resolved());
    assert!(
        changes
            .events
            .iter()
            .any(|e| matches!(e, WorkEvent::CommentThreadReopened { .. }))
    );
}

#[test]
fn assign_from_a_comment_lands_in_my_work_until_resolved() {
    let mut c = Comments::new();
    let at = BlockRef::new(plan_doc(), "blk-bbbbbbbb").unwrap();
    let mut draft =
        NewComment::new(cid(1), Target::block(at), "Can you size this?");
    draft.quote = Some("Rate limits drop bursts".into());
    draft.assign_to = Some(user("dev"));
    let changes = c.add(draft, &[user("mara")], &as_user("priya")).unwrap();
    let (recipients, assignee) = added(&changes);
    assert_eq!(assignee.as_deref(), Some("dev"));
    assert_eq!(recipients, ["dev", "mara"]);

    let items = c.assignments(&user("dev"));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].summary, "Rate limits drop bursts");
    assert_eq!(items[0].assigned_by, Some(user("priya")));
    assert!(c.assignments(&user("priya")).is_empty());

    // Reassigning moves the item; resolving completes it.
    let _ = c
        .assign(&cid(1), Some(user("priya")), &as_user("dev"))
        .unwrap();
    assert!(c.assignments(&user("dev")).is_empty());
    let changes = c.resolve(&cid(1), true, &as_user("priya")).unwrap();
    assert!(c.assignments(&user("priya")).is_empty());
    assert!(matches!(
        changes.events.first(),
        Some(
            WorkEvent::CommentThreadResolved { assignee: Some(a), .. }
        ) if *a == user("priya")
    ));
    // Resolving twice changes nothing.
    assert_eq!(
        c.resolve(&cid(1), true, &as_user("priya")).unwrap(),
        vellum_work::CommentChanges::default()
    );
}

#[test]
fn only_authors_edit_and_deletes_keep_reply_context() {
    let mut c = Comments::new();
    let target = Target::doc(plan_doc());
    let _ = c
        .add(
            NewComment::new(cid(1), target, "First"),
            &[],
            &as_user("priya"),
        )
        .unwrap();
    let _ = c
        .add(
            NewComment::reply(cid(2), cid(1), "Second"),
            &[],
            &as_user("dev"),
        )
        .unwrap();

    let err = c.edit(&cid(1), "Hijacked", &as_user("dev")).unwrap_err();
    assert!(matches!(err, WorkError::InvalidState { .. }));
    let changes = c
        .edit(&cid(1), "First, @mara please check", &as_user("priya"))
        .unwrap();
    assert!(matches!(
        changes.events.first(),
        Some(
            WorkEvent::CommentEdited { new_mentions, .. }
        ) if *new_mentions == [user("mara")]
    ));
    assert!(c.thread(&cid(1)).unwrap().comments[0].edited_at.is_some());

    // Someone else cannot delete; a manager can.
    assert!(c.delete(&cid(2), false, &as_user("priya")).is_err());
    // The root goes first: it stays as a marker because a reply remains.
    let changes = c.delete(&cid(1), false, &as_user("priya")).unwrap();
    assert!(changes.removed_paths.is_empty());
    let thread = c.thread(&cid(1)).unwrap();
    assert!(thread.comments[0].deleted && thread.comments[0].body.is_empty());
    // The last live comment goes: the thread file goes too.
    let changes = c.delete(&cid(2), true, &as_user("mara")).unwrap();
    assert_eq!(changes.removed_paths.len(), 1);
    assert!(c.thread(&cid(1)).is_none());
    assert!(c.on(&plan_doc().to_string()).is_empty());
}

#[test]
fn reactions_toggle_once_per_person() {
    let mut c = Comments::new();
    let _ = c
        .add(
            NewComment::new(cid(1), Target::doc(plan_doc()), "Ship it"),
            &[],
            &as_user("priya"),
        )
        .unwrap();
    assert_eq!(
        c.react(&cid(1), "🎉", true, &as_user("dev"))
            .unwrap()
            .events
            .len(),
        1
    );
    assert!(
        c.react(&cid(1), "🎉", true, &as_user("dev"))
            .unwrap()
            .events
            .is_empty()
    );
    let _ = c.react(&cid(1), "🎉", true, &as_user("mara")).unwrap();
    assert_eq!(
        c.thread(&cid(1)).unwrap().comments[0].reactions["🎉"].len(),
        2
    );
    let _ = c.react(&cid(1), "🎉", false, &as_user("dev")).unwrap();
    let _ = c.react(&cid(1), "🎉", false, &as_user("mara")).unwrap();
    assert!(c.thread(&cid(1)).unwrap().comments[0].reactions.is_empty());
    assert!(
        c.react(&cid(1), "two words", true, &as_user("dev"))
            .is_err()
    );
}

#[test]
fn bad_input_changes_nothing() {
    let mut c = Comments::new();
    let target = Target::doc(plan_doc());
    assert!(
        c.add(
            NewComment::new(cid(1), target.clone(), "  "),
            &[],
            &as_user("dev")
        )
        .is_err()
    );
    let nobody = Ctx {
        actor: None,
        ..as_user("dev")
    };
    assert!(
        c.add(NewComment::new(cid(1), target.clone(), "Hi"), &[], &nobody)
            .is_err()
    );
    assert!(
        c.add(
            NewComment::reply(cid(1), cid(9), "Hi"),
            &[],
            &as_user("dev")
        )
        .is_err()
    );
    let bad_block = Target::Doc {
        doc: plan_doc(),
        block: Some("blk-XX".into()),
    };
    assert!(
        c.add(
            NewComment::new(cid(1), bad_block, "Hi"),
            &[],
            &as_user("dev")
        )
        .is_err()
    );
    assert!(c.thread(&cid(1)).is_none());
    let _ = c
        .add(
            NewComment::new(cid(1), target.clone(), "Hi"),
            &[],
            &as_user("dev"),
        )
        .unwrap();
    assert!(
        c.add(
            NewComment::new(cid(1), target, "Again"),
            &[],
            &as_user("dev")
        )
        .is_err()
    );
}

#[test]
fn thread_files_round_trip_and_rebuild() {
    let mut c = Comments::new();
    let at = BlockRef::new(plan_doc(), "blk-bbbbbbbb").unwrap();
    let mut draft = NewComment::new(
        cid(1),
        Target::block(at),
        "Is the limit per user?\n\n- yes\n- no",
    );
    draft.authored_by = AuthoredBy::Model;
    draft.assign_to = Some(user("dev"));
    let changes = c.add(draft, &[], &as_user("priya")).unwrap();
    let _ = c.react(&cid(1), "+1", true, &as_user("dev")).unwrap();
    let thread = c.thread(&cid(1)).unwrap();
    assert!(changes.threads.contains(&cid(1)));
    assert_eq!(
        thread.path(),
        format!(".comments/{}/{}.yaml", plan_doc(), cid(1))
    );
    let text = thread.to_yaml().unwrap();
    assert!(text.contains("authored_by: model"), "{text}");
    let back = Thread::from_yaml(&thread.path(), &text).unwrap();
    assert_eq!(&back, thread);

    let mut rebuilt = Comments::new();
    rebuilt.load(back.clone()).unwrap();
    assert_eq!(rebuilt.assignments(&user("dev")).len(), 1);
    assert!(rebuilt.load(back).is_err(), "IDs are unique");
    assert!(Thread::from_yaml(".comments/elsewhere/x.yaml", &text).is_err());
}

#[test]
fn orphaned_threads_are_the_open_ones_whose_block_is_gone() {
    let mut c = Comments::new();
    for (n, block) in [
        (1, "blk-aaaaaaaa"),
        (2, "blk-bbbbbbbb"),
        (3, "blk-dddddddd"),
    ] {
        let at = BlockRef::new(plan_doc(), block).unwrap();
        let _ = c
            .add(
                NewComment::new(cid(n), Target::block(at), "Note"),
                &[],
                &as_user("dev"),
            )
            .unwrap();
    }
    let _ = c.resolve(&cid(1), true, &as_user("dev")).unwrap();
    let live: BTreeSet<&str> = vellum_work::mention::blocks(PLAN)
        .iter()
        .filter_map(|b| b.id)
        .collect();
    let orphans: Vec<CommentId> = c
        .orphaned(&plan_doc(), &live)
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(orphans, [cid(3)]);
}

#[test]
fn comment_targets_resolve_keys_aliases_blocks_and_docs() {
    let mut g = northstar();
    assert_eq!(
        g.comment_target("ENG-1").unwrap(),
        Target::Issue { issue: iid(1) }
    );
    let _ = g.move_issue(&iid(1), &tk("OPS"), &as_user("mara")).unwrap();
    assert_eq!(
        g.comment_target("ENG-1").unwrap(),
        Target::Issue { issue: iid(1) },
        "an old key still resolves"
    );
    let block = format!("{}#blk-bbbbbbbb", plan_doc());
    assert!(matches!(
        g.comment_target(&block).unwrap(),
        Target::Doc { block: Some(b), .. } if b == "blk-bbbbbbbb"
    ));
    assert_eq!(
        g.comment_target(&plan_doc().to_string()).unwrap(),
        Target::doc(plan_doc())
    );
    assert!(matches!(
        g.comment_target("ENG-99"),
        Err(WorkError::UnknownIssue(_))
    ));
    assert!(g.comment_target("not a target").is_err());
}

#[test]
fn issue_from_a_block_writes_a_chip_and_a_source_link() {
    let mut g = northstar();
    let at = BlockRef::new(plan_doc(), "blk-bbbbbbbb").unwrap();
    let draft = NewIssue::new(iid(2), tk("ENG"), "Burst limits for mobile");

    // A block that is not there numbers nothing.
    let missing = BlockRef::new(plan_doc(), "blk-zzzzzzzz").unwrap();
    assert!(
        g.issue_from_block(
            draft.clone(),
            &missing,
            None,
            PLAN,
            &as_user("priya")
        )
        .is_err()
    );
    assert!(g.issue(&iid(2)).is_none());

    let (changes, doc) = g
        .issue_from_block(
            draft,
            &at,
            Some("Rate limits drop bursts"),
            PLAN,
            &as_user("priya"),
        )
        .unwrap();
    assert!(changes.issues.contains(&iid(2)));
    assert!(
        doc.contains(
            "Rate limits drop bursts from the mobile app. ENG-2 ^blk-bbbbbbbb"
        ),
        "{doc}"
    );
    let issue = g.issue(&iid(2)).unwrap();
    assert_eq!(doclink::source_of(issue), Some(at.clone()));
    // Writing the chip again is a no-op.
    let key = issue.key();
    assert_eq!(
        doclink::insert_chip(&doc, "blk-bbbbbbbb", &key).unwrap(),
        doc
    );

    // Back-links: every chip in the page, with its block.
    let links = g.backlinks(plan_doc(), &doc);
    let found: Vec<(IssueId, Option<&str>)> = links
        .iter()
        .map(|l| (l.issue, l.block.as_deref()))
        .collect();
    assert_eq!(
        found,
        [
            (iid(2), Some("blk-bbbbbbbb")),
            (iid(1), Some("blk-cccccccc"))
        ]
    );
}

#[test]
fn doc_from_issue_links_both_ways() {
    let mut g = northstar();
    let issue = g.issue(&iid(1)).unwrap().clone();
    let draft = doclink::doc_draft(&issue);
    assert_eq!(draft.title, "Retry storms");
    assert!(draft.body.starts_with("Created from ENG-1."));
    let new_doc: DocId = "01JB3K9Q7Z5N8M2X4V6T0R1Y40".parse().unwrap();
    let _ = g
        .link_doc(&iid(1), new_doc, &draft.title, &as_user("dev"))
        .unwrap();
    let links = &g.issue(&iid(1)).unwrap().links;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, format!("vellum://doc/{new_doc}"));
    assert_eq!(links[0].kind.as_deref(), Some(doclink::DOC));
    assert_eq!(g.backlinks(new_doc, &draft.body)[0].issue, iid(1));
}

#[test]
fn project_pages_embed_issue_views() {
    let mut g = northstar();
    for n in 2..=4 {
        let _ = g
            .create_issue(
                NewIssue::new(iid(n), tk("ENG"), "More work"),
                &as_user("dev"),
            )
            .unwrap();
    }
    let _ = g
        .create_issue(
            NewIssue::new(iid(5), tk("OPS"), "Pager"),
            &as_user("dev"),
        )
        .unwrap();
    let embed = IssueEmbed::from_yaml(
        "filter:\n  field: team\n  op: is\n  values: [ENG]\nlimit: 3\n",
    )
    .unwrap();
    let groups = g
        .embed(&embed, None, &user("dev"), &as_user("dev"))
        .unwrap();
    let shown: usize = groups.iter().map(|gr| gr.issues.len()).sum();
    assert_eq!(shown, 3);
    assert!(
        groups
            .iter()
            .flat_map(|gr| &gr.issues)
            .all(|i| *i != iid(5))
    );

    assert!(
        IssueEmbed::from_yaml("layout: board\n").is_err(),
        "needs a view or filter"
    );
    assert!(
        IssueEmbed::from_yaml("filter: {}\ncolour: red\n").is_err(),
        "unknown key"
    );
    let named =
        IssueEmbed::from_yaml("view: 01JB3K9Q7Z5N8M2X4V6T0R1Y41\n").unwrap();
    assert!(
        g.embed(&named, None, &user("dev"), &as_user("dev"))
            .is_err()
    );
}

#[test]
fn mentions_wait_in_my_work_until_answered_or_resolved() {
    let mut c = Comments::new();
    let at = BlockRef::new(plan_doc(), "blk-bbbbbbbb").unwrap();
    let draft = NewComment::new(
        cid(1),
        Target::block(at),
        "@dev can you check the limits?",
    );
    let _ = c.add(draft, &[], &as_user("priya")).unwrap();

    let items = c.mentions_of(&user("dev"));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].comment, cid(1));
    assert_eq!(items[0].author, user("priya"));
    assert_eq!(items[0].summary, "@dev can you check the limits?");
    // The author is never their own mention, and others see nothing.
    assert!(c.mentions_of(&user("priya")).is_empty());
    assert!(c.mentions_of(&user("mara")).is_empty());

    // A reply from dev answers it; a later mention brings it back.
    let reply = NewComment::reply(cid(2), cid(1), "Looking now.");
    let _ = c.add(reply, &[], &as_user("dev")).unwrap();
    assert!(c.mentions_of(&user("dev")).is_empty());
    let again = NewComment::reply(cid(3), cid(1), "Thanks @dev, and @mara too");
    let _ = c.add(again, &[], &as_user("priya")).unwrap();
    assert_eq!(c.mentions_of(&user("dev"))[0].comment, cid(3));
    assert_eq!(c.mentions_of(&user("mara"))[0].comment, cid(3));

    // Resolving the thread clears it for everyone.
    let _ = c.resolve(&cid(1), true, &as_user("priya")).unwrap();
    assert!(c.mentions_of(&user("dev")).is_empty());
    assert!(c.mentions_of(&user("mara")).is_empty());
}

#[test]
fn a_thread_assigned_to_someone_is_not_also_their_mention() {
    let mut c = Comments::new();
    let mut draft = NewComment::new(
        cid(1),
        Target::doc(plan_doc()),
        "@dev please size this",
    );
    draft.assign_to = Some(user("dev"));
    let _ = c.add(draft, &[], &as_user("priya")).unwrap();
    assert_eq!(c.assignments(&user("dev")).len(), 1);
    assert!(c.mentions_of(&user("dev")).is_empty());
}
