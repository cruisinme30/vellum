---
id: 01M63CFE6D9C4J2CBEQCMZDGBA
title: Git automation
vellum: 1
---

# Git automation

Connect GitHub or GitLab in **Team settings → Git** and issues move by
themselves as code moves.

## Branch names {#branches}

Press `⌘⇧.` on an issue to copy a branch name such as
`dev/eng-142-replay-engine`. Pushing a branch with an issue key moves
the issue to started.

## Pull request words {#magic-words}

Name an issue in a pull request title or description to link it.

::: columns
**Closes the issue when merged**
fixes, closes, resolves, completes, and their other forms such as
"fixed" and "closing".

---

**Links without closing**
ref, references, part of, contributes to, towards.
:::

```text
Fixes ENG-142: stream the replay from the audit log
```

## Automation {#automation}

Each team maps pull request events to statuses:

| Event | Default status |
| --- | --- |
| Branch pushed | In progress |
| Pull request opened | In review |
| Pull request approved | Ready to merge |
| Pull request merged | Done |
| Pull request closed without merging | To do |

An issue linked to several pull requests completes only when all of
them are merged. The team key in the branch or pull request must match
the issue's team.
