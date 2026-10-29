---
id: 01M63FEVM1FK03SGABQRXEY66Y
title: "ADR 0034: The Work service"
status: verified
owner: "@cruisinme30"
tags: [adr, work, api]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D135, D136, D137
- Requirements: R-WK-13, R-WK-15, R-WK-16
- Issue: #258, #319
- Pull requests: #275, #324

:::

## Context

The Work engine (`vellum-work`, ADR 0016 and ADR 0033) has no I/O. The
gateway serves whatever a crate registers through
`vellum_contracts::Service` (#39) as REST, OpenAPI and GraphQL, so Work
needs a service crate that owns its files, maps signed-in users to
handles, and registers every command and query. It is the first crate on
the seam.

## Decisions

### D135: one service per workspace, files are the truth

- `WorkService::open` lists `teams/`, `projects/` and `.comments/` in the
  workspace's blob store and rebuilds the graph and comments from them.
  A file that does not parse is logged and skipped.
- Every command runs under one lock: the engine validates and applies,
  the changed files are written, then events go to the `EventSink`.
- If a write fails, memory goes back to its copy from before the
  command, and the caller gets `unavailable`.
- Project pages keep the path they were loaded from; a new title never
  moves the file.

### D136: the API surface and its scopes

- 24 operations named `work.<verb>_<noun>` under `/v1/work/`: teams,
  inbox, cycles, issues (query, get, create, update, move, triage), My
  Issues, "assign to" items, projects and comments.
- Reads need `read`; issue, project and comment writes need `work`; team
  configuration needs `admin` and a sign-in within 10 minutes.
- Callers act as their Work handle from the `Directory` port. An account
  without a handle is refused with `permission_denied`.
- API shapes are separate from file shapes: IDs, keys and times are
  strings, and an empty string clears a field in an update.
- Admins (`admin` scope) may delete any comment; others only their own.

### D137: what Work leaves to its neighbours

- Documents belong to the drive: creating an issue from a block returns
  the chip (`doc`, `block`, `key`) for the caller to write into the page.
- The jobs service calls `WorkService::run_daily` per team time zone for
  cycles, rollover, burn-up and SLA risk notices.
- The event log adapter behind `EventSink` feeds webhooks,
  notifications, Git automation and My Work.

## Alternatives

- **Separate REST handlers and a GraphQL schema**: two surfaces to keep
  in step; the seam gives both from one list.
- **Exposing the engine's file types directly**: every new front matter
  key would change the API, and custom `serde` code would leak into schemas.
- **Writing files outside the lock**: faster, but two commands could
  interleave writes and leave the disk out of step with memory.
- **Refusing to start on a bad file**: one hand edit would take Work
  down for everyone.

## Consequences

- The CLI registers `WorkService` with the gateway and wires identity,
  the event log and the clock.
- One lock per workspace bounds write throughput; revisit with
  per-team locks if large workspaces need it.
- Cloning the state for rollback costs memory per write; revisit with
  undo logs if workspaces grow past tens of thousands of issues.
- Intake forms and triage suggestions are operations like any other.
  Submitting a form needs the `work` scope; anonymous submission
  through a public form's share link waits on share links and rate
  limits in the gateway. A suggestion is stored, and only a person's
  accept applies it.
- Insights is one read-only query. The team key narrows the caller's
  filter, so a dashboard asks per team without writing filter JSON.
