---
id: 01M63CFE6E892FX5TJPFV635MJ
title: Connect Claude and other models
vellum: 1
---

# Connect Claude and other models

Vellum has an MCP server built in. Connect Claude Desktop, Claude Code,
Cursor or any MCP client, and the model can search, read and draft in
Vellum under your own permissions: it sees exactly what you can see and
nothing more.

## Connect in one minute {#connect}

1. Open **Settings → AI** and pick your client.
2. Copy the snippet Vellum shows into the client's settings.
3. Select **Test connection**.

For a local Vellum, the client runs `vellum mcp`:

```json
{ "mcpServers": { "vellum": { "command": "vellum", "args": ["mcp"] } } }
```

For a server, the client connects over HTTPS and signs you in with
OAuth:

```json
{ "mcpServers": { "vellum": { "url": "https://vellum.northstar.dev/mcp" } } }
```

## What the model can do {#tools}

| Tool | Does |
| --- | --- |
| `search` | Finds pages, rows and issues, with block-level citations |
| `get_doc`, `get_blocks`, `get_outline` | Reads a page, some blocks or its outline |
| `create_draft`, `propose_edit` | Writes a new page or a change as a draft for you to review |
| `create_issue`, `update_issue`, `add_comment` | Works with issues and comments |
| `create_task` | Adds a task to My Work |
| `query_collection`, `upsert_rows` | Reads and updates collection rows |
| `export` | Exports a page or book |
| `list_changes` | Lists what changed since a time |

Built-in prompts include summarise a space, write a PRD, weekly status,
verify a page and triage an issue.

## Scopes {#scopes}

When you connect, you choose what the client may do:

::: facts

- read: search and read
- write:draft: create drafts and proposals (the default)
- write:direct: change pages without review
- work: create and update issues and tasks
- admin: workspace administration

:::

Every call is recorded in the audit log, and you can revoke a client at
any time in **Settings → AI**.

## Private spaces {#private}

An end-to-end encrypted private space is sealed to its members' devices,
so the server cannot search it or pass it to a model. Your own computer
can: `vellum mcp --private` opens the pages sealed to this device, in
memory only, and serves them to your AI client read-only.

1. In the desktop app, open **Settings → Private spaces** and choose
   **Enrol this device** on the space. (From a terminal:
   `vellum private enrol --server <url> --space <id> --label "My laptop"`
   with an API token in `VELLUM_TOKEN`.)
2. On a device that can already open the space, choose **Reseal pages
   for every device**, so existing pages open on the new one too.
3. Add Vellum to your client with the space's ID and your server:

```json
{ "mcpServers": { "vellum-private": { "command": "vellum",
  "args": ["mcp", "--private", "--server", "https://vellum.northstar.dev",
           "--space", "board=01JDB0ARD00000000000000000"],
  "env": { "VELLUM_TOKEN": "<an API token>" } } } }
```

Without `--server`, give the space's folder in the data folder instead,
such as `board=drives/shared/board`.

The model can search, read pages and blocks, and cite them. It cannot
write, comment or export, and nothing it reads leaves your computer
except through the client you connected. The device key stays in your
operating system's keychain.

## For other tools {#markdown}

Any page's Markdown is at its URL plus `?format=md`, and `/llms.txt`
lists what a model may read.
