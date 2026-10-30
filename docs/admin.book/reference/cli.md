---
id: 01M63CFE62S9ZP4WVEVXEQRN7D
title: Command reference
vellum: 1
---

# Command reference

`vellum serve` and the `vellum admin` commands read the server's settings
from `--config <file>` (or `VELLUM_CONFIG`) and `VELLUM_*` variables, such
as `VELLUM_DATA_DIR` for the data folder. `vellum <command> --help` lists
each command's options.

| Command | Does |
| --- | --- |
| `vellum serve` | Runs the server: web app, API, MCP endpoint and background jobs |
| `vellum docs` | Lists the help books built into this binary, or prints a page |
| `vellum mcp` | Runs an MCP server over standard input and output: `--demo` serves the Northstar demo, `--private` serves the end-to-end encrypted spaces sealed to this device, read-only, from the data folder or (with `--server`) from a Vellum server. For the workspace itself, point the client at the server's `/mcp` address |
| `vellum private key` | Prints this device's public key and fingerprint, for enrolling it in private spaces |
| `vellum private enrol`, `vellum private reseal` | Enrols this device in a private space on a server, and seals the space's pages to its current devices after one joins or leaves, readable pages included; new device keys need `--trust <fingerprint>` (token in `VELLUM_TOKEN`) |
| `vellum reindex` | Rebuilds the index from the files |
| `vellum admin backup`, `vellum admin backups`, `vellum admin restore` | Takes a backup now, lists backups, and restores one into an empty folder |
| `vellum admin break-glass` | Issues a one-time code for a 15-minute administrator session when no administrator can sign in |
| `vellum admin move-space` | Moves a space to another shard |
| `vellum profile` | Records a flame graph of a running server |
| `vellum budgets` | Fails when a route's p95 latency is over its budget |
| `vellum openapi`, `vellum graphql-schema` | Prints the API contracts |

## Reading the help books {#docs}

```bash
vellum docs                          # list the books and their pages
vellum docs user                     # the user guide's table of contents
vellum docs user writing/text.md     # print one page as Markdown
vellum docs --help-id editor.slash-menu
```

The books are the same as the ones published on the website, at the
version of the binary you are running.
