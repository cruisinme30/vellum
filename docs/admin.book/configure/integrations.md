---
id: 01M63CFE61SYK94HT651XPNK20
title: Integrations
vellum: 1
---

# Integrations

| Integration | Where | Does |
| --- | --- | --- |
| Email (SMTP) | Admin → Email | Invitations, notifications, digests, task capture |
| GitHub, GitLab | Team settings → Git | Issue automation from branches and pull requests |
| Slack | Admin → Integrations | Notifications, issue intake, task capture |
| Sentry | Team settings → Intake | Creates issues from new errors |
| Kroki | Admin → Diagrams | D2, PlantUML, Graphviz, Excalidraw and draw.io |
| Webhooks | Admin → Webhooks | Signed HTTP calls on issue, comment, project, cycle and other events |
| Calendar | My Work | Per-user ICS feed and CalDAV |

## Webhooks {#webhooks}

Each webhook is signed with HMAC-SHA256 using a secret you set. Vellum
retries failed deliveries with backoff and shows the last deliveries
and responses so you can debug a receiver. The
[developer guide](../../developer.book/api/webhooks.md) documents the
payloads.

## Outbound requests {#ssrf}

Vellum fetches remote content only for link previews, embeds, webhooks
and Kroki. It refuses private and link-local addresses unless you allow
them in **Admin → Network**, so a page cannot make the server call
your internal services.
