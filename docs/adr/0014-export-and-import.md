---
id: 01M63G5E4VEK89AK6JAHGZTNP0
title: "ADR 0014: Export through one flow, and imports that land as Vellum files"
status: verified
owner: "@cruisinme30"
tags: [adr, export, import]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D70, D71, D72, D229, D230, D231
- Requirements: R-EX, R-IM, R-BK-5, NFR
- Issue: #119, #120, #121, #122, #123, #124, #506, #78
- Pull requests: #286

:::

## Context

People must get their pages out of Vellum as a print-quality PDF, a Word
file, a Google Doc, Markdown, HTML or a zip, and must be able to bring a
whole workspace in from Word, Google Docs, Markdown folders, Notion,
Confluence, Outline and Docmost. Charts and diagrams have to stay sharp
(vector), live blocks have to show what they show today, and a 30-page
PDF has to render in under 3 seconds at p95. Books (ADR 0013) need a
whole book, a subtree or one section exported with page-numbered
cross-references.

## Decisions

### D70: one lowering step and a closed Typst world for print

- Every export first lowers the parsed page into one intermediate flow
  (`vellum-export::flow`): headings, paragraphs, tables, figures and
  cross-references, with live blocks already frozen to their current
  state. Each backend (PDF, Word, Google Docs, HTML, screen) reads only
  that flow, so a block is interpreted once and looks the same everywhere.
- The print PDF is rendered by Typst in-process. Our own template is the
  only Typst source; the page content goes in as JSON data, never as
  Typst markup, so nothing a user writes can run as Typst code.
- The Typst world is closed: bundled fonts only, no file, package or
  network access. That keeps output reproducible and keeps unmaintained
  parts of Typst's dependency tree (bibliographies, YAML) unreachable.
- Charts and diagrams are drawn as SVG and embedded as vectors in PDF and
  Word. A looks-like-the-screen PDF is optional, through a Gotenberg
  sidecar run without outbound network.
- Why: one renderer we control is fast (a 34-page page renders in about
  0.4 s in a release build) and has no browser to secure.

### D71: our own Word writer instead of a library

- Word files are written by a small OOXML writer in the crate, with
  real heading and list styles, a table-of-contents field, `PAGEREF` cross-references,
  SVG images (with a PNG fallback) and equations as Office Math.
- Google Docs export uploads that Word file to Drive with conversion, so
  Google Docs and Word share one code path.
- Why: `docx-rs` pinned `zip` 8 with open advisories and supports
  neither SVG images nor Office Math.

### D72: imports produce ordinary Vellum files with fresh IDs

- Every importer turns its source into the same result: Markdown pages
  with front matter, assets beside them, and databases as collections.
  Nothing is written until the caller stores the result.
- Every page gets a fresh ID. Links between imported pages are rewritten
  to `vellum://doc/<id>`, so they survive later moves and renames.
- Notion and Confluence databases become collections, with row pages as
  rows; Notion's ID suffixes are stripped from names.
- Archives are read with limits (entry count, entry size, total size)
  and unsafe paths are rejected, since an import is untrusted input.
- Collections is a service on the same layer as export, so calculators
  and imported databases reach it through a `Collections` port, which
  `vellum-jobs` answers with the collections engine. Without it,
  calculators show an error and databases stay CSV files beside their
  row pages.
- Book export maps a section's depth to its heading level, rebuilds the
  contents and prints references as "§2.1 Services, p. 5".

### D229: export routes read as the caller and preview as SVG

- `vellum serve` exports at `/api/v1/export`: a page as PDF, Word, HTML or
  Markdown, a book (whole, one section or a subtree), and a page to the
  caller's Google Drive with their own token.
- A page is read through the drive and a book through the books service,
  as the caller: view rights and the `read` scope, and a book export
  leaves out the sections the caller may not see.
- Figures come from the render engine's `Figures::render` (#78). Images
  are the page's own files, read as the caller; outside URLs are never
  fetched and export as alt text.
- The app's CSP allows no frames, so the PDF preview is the first pages
  as SVG from the same Typst layout, shown as images.
- Downloads go out as attachments with `nosniff` and a sandboxing CSP.

### D230: imports are scanned first and land in a new folder

- `/api/v1/import` takes a zip export or a Word file; a Google Doc is
  fetched by file ID with the caller's token. Edit rights on the target
  and `write:direct` are checked before the body is read.
- Every file, uploaded or fetched, goes through `upload::check` (size,
  sniffing, zip-bomb limits, ClamAV failing closed) before an importer
  parses it; then the importer's own archive limits apply.
- `Drive::import` writes the files into a new folder, refusing paths that
  leave it or name Vellum's own files, and indexes them like a copy.

### D231: export and import run on blocking threads behind two slots

- Typesetting and parsing are CPU-bound. They run on blocking threads,
  at most two at a time per replica; a request that waits 30 s for a
  slot gets 503 and retries.

## Alternatives

- **Headless Chromium for every PDF**: slower, a large attack surface,
  and print layout (page numbers in references) is weak.
- **Pandoc for Word and imports**: an external binary and a lossy
  middle format; it does not know callouts, collections or live blocks.
- **Keep source IDs on import**: they collide across workspaces and
  across repeated imports of the same space.
- **Export as a background job with a download link**: the slowest
  export we measured takes seconds, so a direct answer is simpler.
- **pdf.js for the preview**: a large script, and the PDF would still
  need a frame or a blob the CSP refuses.

## Consequences

- New blocks need one lowering rule, and every format gets them.
- Typst upgrades must be checked against the dependency audit, whose
  exceptions are listed with reasons in `deny.toml`.
- Golden text fixtures in `corpus/export` show any change in PDF or Word
  output; rewrite them with `VELLUM_BLESS=1` and review the diff.
- Revisit Word import of equations and footnotes, which become warnings.
- Imported attachments stay beside their pages as plain files; moving
  them under `.attachments` is a follow-up.
- Live blocks (issue chips, collection views) export with a note until
  Work and collections answer the `LiveData` port in serve.
