---
id: 01M63CFE6EC3A4987TVBN7YPWC
title: External edits and Git
vellum: 1
---

# External edits and Git

Your pages are files, so you can edit them with any text editor,
script or sync tool. Vellum notices the change and merges it.

## How merging works {#merge}

Vellum never lets the last write win. When a file changes outside
Vellum while someone is editing it inside, Vellum merges the two
versions line by line. When both changed the same line, it keeps
Vellum's version in the page and saves the other as a conflict copy
next to it:

```text
Roadmap 2027 (conflict 2026-11-03 priya).md
```

Open the copy, move what you need into the page and delete the copy.

## Git {#git}

A drive can live in a Git repository. Vellum commits each batch of
saves and pulls and rebases changes from the remote through the same
merge. Use it for docs that also live next to code; the
[docs-as-code recipe](../cookbook/docs-as-code.md) shows the whole
setup.

## Cloud folders {#cloud}

Google Drive, Dropbox and OneDrive folders sync eventually, so Vellum
always merges what they bring rather than trusting the order changes
arrive in.
