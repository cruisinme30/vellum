---
id: 01MNHCDRMMEBA241Y5XZEXM44X
title: "ADR 0071: Rich blocks and one look across the docs site"
status: accepted
owner: "@cruisinme30"
tags: [adr, docs, site]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D309`, `D310`, `D311`, `D312`
- Issue: `#556`
- Pull requests: the integration batch that closes `#556`

:::

## Context

The website built the help books with unmodified mdBook (ADR 0027,
D111), which knows only CommonMark. Every Mermaid diagram was published
as its source text, and every `:::` callout, card grid, step list and tab
set was unwrapped into plain paragraphs, so the docs showed none of the
rich blocks they teach. The landing page, the docs home and the books
also had three different top bars, so the site did not read as one site.
Gourav asked for architecture and flow diagrams, rich content and one
consistent theme.

## Decisions

### `D309`: rich blocks are drawn by vellum-render at site build

- `crates/vellum-render/examples/site_blocks.rs` rewrites each staged
  page before mdBook runs: every top-level `:::` container and every
  figure fence (Mermaid, `architecture`, charts, timelines, roadmaps,
  risk matrices, boards, calculators, tables, images) becomes the HTML
  Vellum itself renders, on one line that mdBook passes through.
- Plain code fences, the help books' ```` ```markdown ```` examples and
  everything else stay Markdown, so mdBook still builds its outline and
  search index from them.
- The renderer's scoped stylesheet (`render.css`, ADR 0068, D287)
  follows `data-theme` on `<html>`, which `theme.js` now always sets, so
  diagrams switch with the site's light and dark toggle.
- `check-site.sh` fails when a page still shows a Mermaid, architecture
  or chart fence as source.
- Why: the docs show exactly what the app shows, with no second diagram
  pipeline and no client-side Mermaid bundle.

### `D310`: one site bar from one file on every page

- `site/bar.html` is the only markup of the top bar; `build-site.sh`
  puts it into the landing page, the docs home and every book page,
  including the decision records and older releases' books.
- `site/bar.css` styles it in px, so it looks the same in the landing
  page's 16 px rems and mdBook's 10 px rems (D307). It wraps rather than
  pushing off screen (#248). In a book it is fixed above the sidebar, and
  the sidebar and mdBook's menu bar start below it, at the bar's height
  that `theme.js` measures.
- The bar marks the page's own section with `aria-current="page"`.
- `check-site.sh` compares every page's bar with the landing page's,
  ignoring only paths, the current section and the release links.

### `D311`: tab sets become real tabs in the books

- `site/blocks.js` turns each rendered tab set into a WAI-ARIA tablist
  with arrow, Home and End keys. Without the script, and when printed,
  every tab shows one after another, as the renderer writes them.

### `D312`: the guides carry diagrams and screenshots

- Each guide explains its main flows with diagrams: services and sign-in,
  live collaboration, offline sync and export in the developer guide;
  deployment shapes, sign-in, SCIM and backups in the admin guide; issue
  states, drafts, external edits, exports and My Work in the user guide.
- Screenshots of the Northstar demo are copies of the website's, kept
  under each chapter's `assets/` so the in-app guide serves them too.

## Alternatives

- **Mermaid in the browser**: a large script on every page, a second
  renderer that can disagree with the app, and the landing page's
  script policy would need loosening.
- **An mdBook preprocessor plugin**: one more binary to pin and verify;
  the example uses the crate directly and is tested with it.
- **Replacing mdBook's page template**: ties the site to one mdBook
  version's markup; the bar needs only a header partial and CSS.

## Consequences

- The Website workflow now builds one Rust example, cached between runs;
  its timeout grows from 10 to 25 minutes.
- Headings inside a rendered container are not in mdBook's outline or
  search; long sections should stay outside containers.
- A book page without JavaScript assumes the bar's usual height; if the
  links wrap further, the bar can cover the top of the sidebar.
