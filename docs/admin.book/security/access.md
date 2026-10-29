---
id: 01M63CFE61Y6897J5GBSD6Q6BS
title: Access and audit
vellum: 1
---

# Access and audit

## How access is decided {#model}

Access is a set of grants: a person, group or everyone, a role, and a
drive, folder or page. Vellum keeps the effective access of every page
precomputed, so each check is one indexed lookup. Permission changes
take effect everywhere in under a second.

## Admin roles {#roles}

| Role | Can |
| --- | --- |
| Owner | Everything, including transferring ownership and managing keys |
| Admin | Users, groups, sign-in, drives, integrations and settings |
| Auditor | Read the audit log and access reports, change nothing |

Admins do not automatically see content. To open a page an admin must
be granted access like anyone else, or use **Access override**, which
asks for a reason and is recorded in the audit log.

## The audit log {#audit}

**Admin → Audit** lists every sign-in, share, permission change, export,
admin action and AI call, with who, when, from where and the request ID.
The log is append-only. Stream it to your SIEM with
`OTEL_EXPORTER_OTLP_ENDPOINT` or download it as JSON lines.

## Access reports {#reports}

**Admin → Access → Report** answers "who can open this?" for any page
and "what can this person open?" for any user, with the reason for each
answer.
