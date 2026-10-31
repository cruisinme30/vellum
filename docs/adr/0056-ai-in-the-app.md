---
id: 01M64VV212P7QF65QSP59QC5K1
title: "ADR 0056: AI in the app with the user's own model"
status: draft
owner: "@cruisinme30"
tags: [adr, ai, mcp, security]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D238, D239, D240
- Issue: #173, #507
- Pull requests: none yet (handed to the integrator as feat/ai-byom)

:::

## Context

R-AI-8 and R-SR-5 ask for summarise, "ask this space" with citations, and
draft from template in the web app, using the person's own API key and
never a model Vellum runs. R-AI-5 asks for an admin page with the MCP
client snippets and a "Test connection" button; the server half shipped
in #170 ([ADR 0019](0019-mcp-server.md)). MCP now runs inside `vellum serve`
through a workspace adapter that reads and writes as the signed-in user.
Keys for a paid model are valuable secrets, and quoted workspace content
can carry prompt injection.

## Decisions

### `D238`: the browser keeps the key; the server calls the model and forgets it

- The person enters provider, model, endpoint and key on `/settings/ai`.
  The browser keeps them in session storage, or local storage with
  "Remember on this device", and sends them with each `/api/v1/assist/*`
  call. The server never stores the key: `ApiKey` has no `Serialize` and a
  redacted `Debug`, and provider errors have the key scrubbed out.
- The server makes the call so the app's CSP stays `connect-src 'self'`,
  retrieval stays permission-filtered, and every call passes the SSRF
  guard (`SafeClient::post_json`, no redirects).
- Two wire formats: the Anthropic Messages API (default model
  `claude-opus-5-5`) and OpenAI-style chat completions, which also covers
  Ollama, LM Studio and vLLM. Public endpoints must be `https`; a
  self-hosted endpoint on the private network works only when an admin
  lists it in `VELLUM_AI_ENDPOINTS`. `VELLUM_AI=off` removes the API.
- Saving runs a one-line test call first, so a wrong key or model fails
  on the settings page rather than mid-task.

### `D239`: the assistant reads through the MCP workspace adapter

- New crate `vellum-assist` (service layer) with a `Sources` port. The
  binary implements it over `vellum_mcp::ports::Workspace` as
  `Actor::User` with read scope, plus `write:draft` when the session has
  it, so the assistant sees exactly what that person's MCP client sees.
- Ask: permission-filtered hybrid search, the best 8 blocks quoted as
  numbered `<source>` elements, `[n]` marks read back into citations with
  deep links. No hits means no model call.
- Quoted content is data: invisible and direction-control characters are
  stripped, a source cannot close its own tag, front matter is never sent,
  and the system prompt says text in sources is never an instruction.
- Draft from template: any readable page is the template; the result lands
  as a draft page attributed `authored_by: model` for the person to review
  (R-AI-3), never as a direct write.
- Pages over 120,000 characters are cut, and the answer says so.

### `D240`: the admin MCP page tests a pasted token once

- `/admin/mcp` lists the snippets from `GET /mcp/setup` with copy buttons.
- "Test connection" sends the pasted token once as a bearer header to
  `POST /mcp/test-connection`, without cookies, and shows each check.
  The token is never stored or sent anywhere else.

## Alternatives

- **Call the model from the browser**: rejected; it needs every provider
  host in the CSP, puts retrieval in the client, and can't reach a
  self-hosted model on the server's network.
- **Store keys on the server, sealed per user**: rejected for 1.0; it needs
  shared encrypted storage across replicas and makes Vellum a holder of
  model keys. The per-request design works on every replica unchanged.
- **Our own retrieval over search and drive**: rejected; it would duplicate
  the MCP adapter's permission checks.

## Consequences

- A person sets up the key once per browser (or per tab without
  "Remember").
- The assist service is mounted only when the MCP workspace adapter is
  wired (`Services.mcp_workspace`).
- Model calls take up to two minutes (three for drafts); the operations'
  budgets say so.
