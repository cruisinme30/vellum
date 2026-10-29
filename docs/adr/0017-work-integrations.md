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

Brief section 8 asks for Git automation, signed webhooks, intake from
Slack, Sentry, email and forms, imports from Linear, Jira and GitHub
Issues, recurring issues, a prefilled create URL, insights and
AI-suggested triage. Two service-layer crates carry it: `vellum-notify`
delivers (webhooks, inbox, email, web push) and `vellum-integrations`
handles inbound and scheduled work. Both are domain logic behind ports
and reach the issue core only through the `WorkCommands` port.

## Decisions

### D79: Git automation rules

- A key in a branch name or pull request title closes; in descriptions
  and commits only magic words count (close, fix, resolve, complete; ref,
  part of, related to, contributes to, towards). Linking beats closing.
- Each repository lists the teams it may act on; other keys are ignored,
  so `UTF-8` or `PR-12` never touch an issue.
- Each team maps five moments (branch, opened, approved, merged, closed
  unmerged) to one of its states or to nothing.
- An issue reaches the merged state only when every open closing pull
  request has merged. A closing commit on the default branch counts.
- Automation never reopens a closed issue or moves one backwards, and
  linking references never change state.

### D80: outbound delivery

- Webhooks carry `Vellum-Signature: t=<unix>,v1=<hex HMAC-SHA256>` over
  `"{t}.{body}"`; both secrets sign during rotation; receivers allow five
  minutes of skew. Secrets are 32 random bytes shown once.
- "All public teams" never covers a private team; only naming it does.
- Eight attempts over about 45 hours, honouring `Retry-After` up to an
  hour. 410 removes a webhook; 404 or 410 removes a push subscription;
  ten failures over 72 hours pause an endpoint.
- Every outbound URL passes the shared SSRF guard in `vellum-security`
  on save and again on the resolved addresses, with no redirects.
- Inbox always, push at once, email after ten minutes unless read.
  Urgent kinds skip the wait and pierce mutes. Actors are not notified
  of their own actions.
- Email threads per subject and has one-click unsubscribe (RFC 8058).
  Push payloads are encrypted with `aes128gcm` (RFC 8291) and VAPID.

### D81: inbound and scheduled work

- Every inbound webhook is verified over the raw body in constant time
  before parsing. Intake builds drafts for the team's triage inbox, each
  with a source ID so retries never file twice.
- Email replies go to `reply+KEY.user.tag@`, where the tag is an HMAC,
  so a reply cannot be forged for another issue or user. Automatic mail
  is dropped to stop loops.
- Imports create issues oldest first, keep source IDs as aliases, skip
  what exists and link relations in a second pass. Unknown people stay
  unassigned with a warning.
- A recurring issue is created at 00:01 team time on the day after the
  previous due date; after an outage only the latest missed one is.
- The create URL fills a form and never creates. Insights use
  nearest-rank percentiles and count issues missing a timestamp as
  skipped, never as zero.
- Triage suggestions come from the user's own model, keep only offered
  values and are never applied automatically. Duplicates are found by
  local trigram similarity.

## Alternatives

- **One crate for everything**: mixes people-facing delivery with
  tracker automation, so it has two reasons to change.
- **Polling provider APIs for pull request state**: needs a token per
  repository; webhooks already carry what the rules need.
- **Calling `vellum-work` directly**: ties these crates to the core's
  internals and slows their tests; a port keeps them independent.

## Consequences

- The issue core needs no provider knowledge; it receives state changes
  with a reason through `WorkCommands`, by a bot user audited like a
  person.
- Receivers keep the raw request body, so they mount on the gateway as
  raw routes rather than typed operations.
- Webhook receivers written for other trackers must read
  `Vellum-Signature`; the timestamp-plus-HMAC shape is common.
- HTTP routes and the `vellum-work` adapter follow once the gateway and
  #207 merge.
