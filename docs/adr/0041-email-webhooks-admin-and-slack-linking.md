---
id: 01M67BJDFYX6D0AQSGFY1PMSF0
title: "ADR 0041: Notification email, the webhooks admin and Slack linking"
status: verified
owner: "@cruisinme30"
tags: [adr, integrations, notifications]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D176, D177, D262, D263
- Issue: #164, #149, #163
- Pull requests: to be linked when merged

:::

## Context

ADR 0017 built the notifications and webhooks domain: signed payloads,
retries, the delivery plan with its email delay, and an email encoder.
`vellum serve` used only the inbox and web push. It skipped every
email, had no webhook subscriptions, and had no way for a Slack user
to link their account for "Save as task". My Work (#472, ADR 0042)
schedules reminders and the morning digest as notifications. This ADR
records how they reach email, how admins run webhooks, and how Slack
links.

## Decisions

### D262: email is a queue in the notify state, sent by one timer

- The dispatcher queues each planned email delivery per person in
  `.vellum/notify.json`. A burst about one subject collapses into one
  email with the newest text, sent at the earliest planned time.
- When an email falls due it is dropped if the inbox line was read, the
  subject muted or email switched off for the kind. A snooze moves it to
  the snooze's end. A failed send is retried with backoff, five times.
- `VELLUM_SMTP_URL` (`smtp://`, `smtps://`, or `file://` for `.eml`
  files) and `VELLUM_MAIL_FROM` configure the transport, through lettre
  over rustls. Without them, email stays off as before.
- With a transport, the server calls `NotifyStore::enable_email`, and
  every dispatcher queues email. So My Work's reminders and digest reach
  email with no code of their own.
- Every email has a signed one-click unsubscribe link (RFC 8058). `GET
  /unsubscribe` shows a button and `POST` mutes the subject. The HMAC
  key lives in the notify state.

### D263: admins manage webhooks under /v1/admin/webhooks

- `webhooks.*` operations need the admin scope. Operations: list,
  create, update, delete, rotate the secret, test (a signed `ping`),
  the last 100 deliveries, and redeliver.
- The secret is shown once, at create or rotate. After a rotation the
  old secret signs too (a second `v1=`) for 24 hours.
- URLs pass the SSRF policy of ADR 0017 when saved, and again at
  connect time. The state is `.vellum/webhooks.json`, mode 0600.
- Retries follow the RetryPolicy (about 45 hours). A hook that keeps
  failing for three days is paused, with its reason shown, and its
  queue is dropped. Turning it back on clears the failures.

### D176: Work events become webhook events; one replica delivers

- `work::webhook_events` maps issue, cycle, SLA, project, project
  update, label and issue comment events. Updates carry
  `updated_from`.
- Comments on documents are never sent. A document's readers come
  from its own permissions, which a webhook cannot check.
- Work teams have no private flag yet, so hooks covering all public
  teams get every team.
- Webhook deliveries and queued emails are sent by a 30-second timer
  under the `notify.deliver` cluster lease (ADR 0038). It runs on one
  replica.

### D177: Slack users link themselves with a signed code

- An unlinked user's "Save as task" gets a link to
  `/my-work?slack_link=CODE`. The code is an HMAC of the Slack team,
  the Slack user and an expiry, keyed by the Slack signing secret. It is
  valid for 15 minutes.
- My Work asks before it links. `POST /v1/my-work/slack-link` then
  links that Slack user to the signed-in person; `DELETE` unlinks.
- Replies go to Slack's `response_url`, but only under
  `https://hooks.slack.com/`, with no redirects and within 2.5 s. They
  also go in the response body.

## Alternatives

- **A second email digest of unread notifications**: My Work's morning
  digest (ADR 0042) already covers the day. A second email and a second
  digest-time setting would be noise, so it was dropped.
- **Email in the job queue table**: the notify state is still per data
  folder (ADR 0017). Moving notify state to the shared database belongs
  with that move.
- **Linking Slack by typing a member ID**: anyone could claim someone
  else's Slack account and receive their saved messages.
- **Sending a document comment's webhook after a permission check**: a
  webhook has no reader to check against.

## Consequences

- Operators set two variables to turn email on. The SMTP URL carries
  the password, so treat it as a secret.
- Admins have a Webhooks screen in the console: health, the last
  delivery, test, rotate, turn on or off, and the delivery log.
- People who use Slack can link themselves. `slack-users.json` is now
  written by Vellum and can still be edited by hand.
- The notify and webhook state is per data folder. With several
  replicas, the replica holding `notify.deliver` sends only its own
  folder's queue. That works on one server and needs the shared-state
  work of #432 for clusters.
