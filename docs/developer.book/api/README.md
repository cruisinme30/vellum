---
id: 01M63CFE65GQD3E9GX2ZASQZJ0
title: REST and GraphQL
vellum: 1
---

# REST and GraphQL

Vellum has two equivalent APIs: REST described by OpenAPI 3.1, and
GraphQL with full mutation coverage. Use whichever suits the client;
both run the same authorization and are audited the same way.

::: facts

- REST base: `/api/v1`, described at `/api/v1/openapi.json`
- GraphQL: `/graphql`, with the schema at `/graphql/schema.graphql`
- TypeScript client: generated from the OpenAPI document and published with each release

:::

## Authentication {#auth}

Send a personal API token as a bearer token. The token's scopes limit
what it can do, on top of the user's own permissions.

```bash
curl -H "Authorization: Bearer $VELLUM_TOKEN" \
  https://vellum.northstar.dev/api/v1/pages/01JB3K9Q7Z5N8M2X4V6T0R1Y3W
```

## Reading a page {#read}

```json
{
  "id": "01JB3K9Q7Z5N8M2X4V6T0R1Y3W",
  "title": "Roadmap 2027",
  "status": "verified",
  "owner": "priya",
  "version": 42,
  "markdown": "---\nid: 01JB3K9Q7Z5N8M2X4V6T0R1Y3W\n…"
}
```

Any page's raw Markdown is also at its web address with `?format=md`.

## Writing {#write}

Every write takes an `Idempotency-Key` header. Sending the same key
twice applies the write once and returns the first result, so clients
can retry safely. Updates take the version they are based on and are
merged three ways if the page has changed since.

## Errors {#errors}

Errors use RFC 9457 problem details and carry the request ID:

```json
{
  "type": "https://vellum.dev/errors/forbidden",
  "title": "You cannot edit this page",
  "status": 403,
  "request_id": "01JBA0C2E4G6J8L0N2Q4S6U8W0"
}
```

## GraphQL {#graphql}

```graphql
query MyIssues {
  viewer {
    issues(filter: { state: { category: { in: [unstarted, started] } } }, first: 20) {
      nodes { key title priority state { name } cycle { name } }
    }
  }
}
```

## Rate limits {#limits}

Limits are per token and per route. Responses carry `RateLimit` headers;
a `429` response says when to retry.
