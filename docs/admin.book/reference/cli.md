---
id: 01M63CFE62S9ZP4WVEVXEQRN7D
title: Command reference
vellum: 1
---

# Command reference

Every command accepts `--config <file>` and `--data-dir <folder>`, and
`--json` for machine-readable output.

| Command | Does |
| --- | --- |
| `vellum serve` | Runs the server |
| `vellum docs` | Lists the help books built into this binary, or prints a page |
| `vellum mcp` | Runs the MCP server over standard input and output |
| `vellum new` | Creates a page, book or collection from the command line |
| `vellum open` | Opens a page in the browser |
| `vellum search` | Searches from the command line |
| `vellum export` | Exports a page or book to PDF, Word, Markdown or HTML |
| `vellum import` | Imports from Notion, Confluence, Google Docs, Obsidian, Outline, Linear, Jira or GitHub Issues |
| `vellum reindex` | Rebuilds the index from the files |
| `vellum backup`, `vellum restore` | Takes and restores backups |
| `vellum keys` | Initialises, rotates and inspects encryption keys |
| `vellum migrate-storage` | Moves a drive to another storage backend |
| `vellum doctor` | Checks the installation and explains problems |
| `vellum profile` | Records a flame graph |

## Reading the help books {#docs}

```bash
vellum docs                          # list the books and their pages
vellum docs user                     # the user guide's table of contents
vellum docs user writing/text.md     # print one page as Markdown
vellum docs --help-id editor.slash-menu
```

The books are the same as the ones published on the website, at the
version of the binary you are running.
