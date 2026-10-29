---
id: 01M63CVKGKJ6V837H4NF4M9WXW
title: "ADR 0017: Work integrations, webhooks and notifications"
status: verified
owner: "@cruisinme30"
tags: [adr, work, integrations, notifications, service]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D79, D80, D81
- Issue: #147, also #144, #145, #146, #149, #150, #151, #164
- Pull requests: #227

:::

## Context

Brief section 8 lists the integrations around issue tracking: Git
automation for GitHub and GitLab, signed webhooks for issue, comment,
project, project update, cycle, label, user and SLA events, intake from
Slack, Sentry, email and forms, imports from Linear, Jira and GitHub
Issues, recurring issues, the prefilled create URL, insights, and
AI-suggested triage through the user's own model. Section 3 names
notifications and webhooks as one of the eleven services. The issue core
lives in `vellum-work` (engine layer); a service never reads another
service's tables.

Two crates carry this work, both in the service layer:

| Crate | Holds |
| --- | --- |
| `vellum-notify` | webhook events, signatures, subscriptions, delivery retries, the in-app inbox plan, email encoding, web push encryption |
| `vellum-integrations` | Git automation, intake, imports, recurring issues, the create URL, insights, triage suggestions |

Both are domain logic behind ports. They open no sockets and read no
tables; adapters supply HTTP, mail, storage and the issue core.

## Options considered

- **One integrations crate with everything**: simplest to start, but it
  would mix people-facing delivery (inbox, email, push) with tracker
  automation and give one crate two reasons to change. Not chosen.
- **Two service crates behind ports** (`vellum-notify` for delivery,
  `vellum-integrations` for inbound and scheduled work): each tests
  without a network, database or issue core. Chosen.
- **Call provider APIs to read pull request state** instead of relying on
  webhooks: needs a token per repository and polling. Not chosen;
  webhooks carry everything the rules need.

## Decisions

### D79: Git automation rules

- **References.** A key in the branch name or pull request title is a
  closing reference. In descriptions and commit messages only magic words
  count: closing words (close, fix, resolve, complete, in every form) and
  linking words (ref, refs, references, part of, related to, contributes
  to, towards). A word applies to the list of keys after it. Keys may be
  written as issue links. An explicit word beats an implicit mention, and
  linking beats closing, so `Part of ENG-142` stops a title mention from
  completing the issue.
- **Team keys must match.** Each connected repository lists the teams it
  may act on; keys of other teams are ignored, so `UTF-8` or `PR-12`
  never touch an issue.
- **Per-team mapping.** Five moments map to one of the team's states or
  to nothing: branch pushed or draft opened (first started state), pull
  request opened (a started state named like "review"), approved (named
  like "ready" or "merge"), merged (first completed state), closed
  without merging (first unstarted state).
- **All pull requests must merge.** An issue's stage is the least
  advanced of its open closing pull requests. It reaches the merged state
  only when every closing pull request has merged; ones closed unmerged
  drop out. A closing commit pushed straight to the default branch counts
  as merged.
- **Never surprising.** Automation never reopens a closed issue, never
  moves an issue backwards, and after "closed unmerged" only moves an
  issue back if automation put it where it is. Linking references never
  change state. A pull request edited to drop a key unlinks it and the
  issue is re-evaluated.

### D80: outbound delivery

- **Signatures.** Every webhook carries
  `Vellum-Signature: t=<unix seconds>,v1=<hex HMAC-SHA256>` over
  `"{t}.{body}"`. Receivers reject timestamps more than five minutes
  away. During secret rotation both secrets sign, so the header carries
  two `v1` values. Secrets are 32 random bytes shown once as `whsec_…`
  and stored sealed. Each request also carries `Vellum-Event` and a
  `Vellum-Delivery` ID that is stable across retries.
- **Scope.** A subscription chooses resources and teams. "All public
  teams" never covers a private team; only naming it does. Events without
  a team, such as users, go to every subscription that wants the resource.
- **Retries.** Eight attempts over about 45 hours (now, 1 minute,
  5 minutes, 30 minutes, 2, 6, 12 and 24 hours); `Retry-After` is
  honoured up to an hour. 408, 429, 5xx and network errors retry; other
  4xx do not. 410 Gone removes the endpoint, and a push service's 404
  removes the push subscription. An endpoint that has failed
  ten deliveries in a row over at least 72 hours is paused and its
  owners are emailed.
- **SSRF.** Every outbound URL goes through the shared guard in
  `vellum-security` when saved and again on the resolved addresses at
  connect time, so DNS rebinding cannot reach the internal network.
- **People.** The inbox gets every notification whose kind is on.
  Web push goes out at once. Email waits ten minutes and is skipped if
  the inbox entry was read, so active users are not emailed about what
  they saw. Urgent kinds (SLA breached, reminders) skip the wait and
  pierce mutes. The actor is never notified of their own action.
- **Email and push formats.** Email is `multipart/alternative`, threads
  per subject through a stable root message ID, and carries one-click
  unsubscribe headers (RFC 8058). Web push payloads are encrypted to the
  browser's key with `aes128gcm` (RFC 8291) and signed with VAPID
  (RFC 8292); the push service never sees notification text.

### D81: inbound and scheduled work

- **Intake lands in triage.** Slack, Sentry, email and form intake build
  issue drafts without a state; the issue core files them in the team's
  triage inbox. Each draft carries its source ID (Sentry issue, email
  message ID, Slack link) so retries never file twice. Every inbound
  webhook is verified over the raw body before parsing: Slack and Sentry
  HMAC signatures, GitHub `X-Hub-Signature-256`, the GitLab token in
  constant time.
- **Email.** Mail to `TEAM@` the intake domain, or `issues+TEAM@`,
  files an issue. Replies to notifications go to
  `reply+KEY.user.tag@`, where the tag is an HMAC of the key and user, so
  a reply becomes a comment by that user and cannot be forged for another
  issue. Automatic mail (`Auto-Submitted`, bulk, lists) is dropped to
  stop loops.
- **Imports are idempotent.** Readers turn Linear CSV, Jira search JSON
  (with Atlassian documents converted to Markdown) and GitHub issue JSON
  into one shape. Issues are created oldest first, source IDs become
  aliases, re-imports skip what exists, and parents and relations are
  linked in a second pass. Unknown people are left unassigned with a
  warning, never guessed.
- **Recurring issues.** The first issue is created when the schedule is
  saved; each next one at 00:01 team time on the day after the previous
  due date. After an outage only the most recent missed occurrence is
  created, and the skipped ones are counted.
- **The create URL never creates.** `/team/KEY/new?title=…` opens a
  filled-in form; bad parameters are dropped and listed, not fatal.
- **Insights.** Lead time is created to completed, cycle time started to
  completed, triage time created to triaged. Percentiles use the
  nearest-rank method. Issues missing a timestamp are counted as skipped,
  never as zero.
- **Triage suggestions are suggestions.** They come from the viewing
  user's own model, receive the issue text as quoted data with invisible
  characters removed, and every value in the answer must be one of the
  choices offered or it is dropped. A person accepts or edits them;
  nothing is applied automatically. Likely duplicates are found locally
  by trigram similarity, so the inbox works without a model too.

## Consequences

- The issue core needs no knowledge of providers: it receives state
  changes with a reason ("GitHub `northstar/api#12` merged") through the
  `WorkCommands` port, and the integration's bot user is authorised and
  audited like a person.
- Receivers must keep the raw request body for signature checks, so they
  mount on the gateway as raw routes rather than typed operations.
- Webhook receivers written for other trackers need a small change to
  read `Vellum-Signature`; the format follows the common
  timestamp-plus-HMAC pattern, so most libraries already support it.

## History

- 2026-10-29: accepted in #227.
