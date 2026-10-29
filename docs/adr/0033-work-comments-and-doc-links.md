---
id: 01M63EP9WPP4GJG45PK4TG8GM8
title: "ADR 0033: Comments and issue ↔ doc links"
status: verified
owner: "@cruisinme30"
tags: [adr, work, comments]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D129, D130, D131
- Requirements: R-WK-2, R-WK-7, R-MW-7
- Issue: #134, #140
- Pull requests: pending

:::

## Context

Issues and documents both need threaded comments with reactions and
mentions (R-WK-2), "assign to" from a comment that lands in My Work
(R-MW-7), and two-way links between issues and document blocks
(R-WK-7, D18). Comments must survive page renames and issue moves, stay
readable as files, and give My Work, notifications and webhooks one
source. `vellum-work` adds a `comment` module and a `doclink` module.

## Decisions

### D129: comment threads are files keyed by object ID

- One thread per file: `.comments/<doc or issue id>/<thread id>.yaml`,
  with the root comment and its replies, oldest first. The thread ID is
  the root comment's ID.
- Threads are filed by the target's ID, not its path, so renaming a
  page or moving an issue to another team never orphans them.
- A thread targets an issue, a page, or one block (`blk-…`) with the
  quoted text. A block thread whose block is gone is listed as
  orphaned, not deleted.
- Replies are one level deep (Google Docs, Linear). Replying to a
  resolved thread reopens it.
- Authors edit and delete their own comments; managers may delete any.
  A deleted root with live replies stays as a marker; a thread with
  nothing left is removed.

### D130: mentions, assignment and notification

- A mention is `@handle` at the start of a word, outside code; emails,
  `@today` and `@me` are not mentions.
- A comment notifies the thread's participants, the target's followers
  (issue subscribers, or the page owner), everyone mentioned and the
  assignee, never the author.
- On an issue, commenting, being mentioned or being assigned subscribes
  that person to the issue.
- "Assign to" gives the whole thread to one person. It is an open My
  Work item until the thread is resolved; reassigning moves the item.
- Reactions are one per person per emoji, at most 20 kinds per comment.

### D131: issue ↔ doc links live in the files people read

- Creating an issue from a block adds a `kind: source` link
  (`vellum://doc/<id>#blk-…`) to the issue and writes the key into the
  block as a chip. The block is checked before the issue is numbered.
- Any issue key written in capitals in a document is a chip. The
  service indexes `WorkGraph::backlinks` per document so an issue lists
  every block that mentions it, aliases included.
- "Doc from issue" starts the page with a chip back to the issue and
  adds a `kind: doc` link to the issue.
- A project page embeds issues with an `::: issues` block whose YAML
  names a saved view or a filter, with optional layout, grouping and a
  limit of at most 200.

## Alternatives

- **Comments inside the target's file**: every comment would rewrite the
  page or issue and race with editing; threads could not be loaded alone.
- **One file per comment**: many tiny files for a short thread, and a
  reply's context spread across them.
- **Comments in a database table**: breaks "every object is a file", and
  exports and Git history would lose discussions.
- **A separate back-link file per issue**: a second copy of facts already
  in the documents, which goes stale on every edit.

## Consequences

- The service loads `.comments/` into `Comments` at start, writes the
  thread files named in `CommentChanges`, then calls
  `WorkGraph::apply_comment` for issue subscriptions.
- My Work reads `Comments::assignments`; notifications and webhooks read
  the comment events.
- Render and the editor show `::: issues` blocks and chips; the service
  enforces permissions on what an embed shows.
