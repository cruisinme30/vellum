---
id: 01M64VTWG19WC7CQX34HQGZMRZ
title: "ADR 0062: Help in the app, versioned docs and recorded videos"
status: verified
owner: "@cruisinme30"
tags: [adr, docs, help]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D256, D257, D258
- Requirements: R-HG-1, R-HG-4, R-HG-5
- Issue: #202, #203
- Pull requests: to be linked when merged

:::

## Context

[ADR 0024](0024-help-books.md) put the help books in the binary and mapped help
IDs to headings, but nothing in the app opened them, the website had no
versions, feedback or edit links, the ten videos had no way to be made, and
translations had no check. The website also published every part's introduction
(each `README.md`) as an empty page with only its title.

## Decisions

### D256: the server renders its own books at /guide

- `vellum serve` mounts `/guide`: `/guide/topics/<help ID>` redirects
  to `/guide/<book>/<page>.html#u-<anchor>`, rendered from the books in
  the binary by `vellum-render` with the theme stylesheet.
- Pages carry no script, links between chapters become `.html`, and every
  path is looked up in the embedded tables, never on disk.
- `cargo xtask help write` writes `web/src/help/topics.gen.ts`; the
  app's `?` links (`HelpLink`, `ViewTitle`) accept only those IDs.
- Against a server a `?` opens `/guide` in a new tab; in the demo it
  opens the published guide.
- Why: the help matches the running version, works offline and shows
  every block exactly as the reader's own pages render it.

### D257: the website keeps release copies and asks for feedback in the open

- `build-site.sh` builds the books of the five newest `v*` tags into
  `docs/v/<tag>/`; the site bar links each guide's other versions.
- Every help page ends with "Was this page helpful?" and "Edit this
  page". Yes and No open a prefilled GitHub issue labelled
  `docs-feedback`; nothing is sent unless the reader submits it, and the
  site has no analytics.
- `README.md` chapters are staged as `index.md` and mdBook runs with
  `create-missing = false`, so a part's introduction is never empty again.
- `check-site.sh` fails a page without the feedback links or with an
  unfilled placeholder.

### D258: videos are recorded tours; translations are checked copies

- A tour in `web/e2e/tours/*.tour.ts` drives the app; each scene's
  caption becomes a WebVTT cue. `pnpm run tours` records 1280×720 video
  and captions; a release re-records them.
- The storyboard for all ten videos is the developer guide's "Video
  storyboard" page.
- Translations live at `docs/i18n/<locale>/<name>.book/`. `cargo xtask
  help` checks the locale tag, and that every translated page keeps its
  English path, `id` and heading anchors.

## Alternatives

- **Help in a side panel inside the app**: the app never injects HTML,
  and a sandboxed frame would need its own styles and CSP exceptions.
- **Analytics for "was this helpful"**: a tracker on a privacy-first
  product's docs; GitHub issues are public and actionable.
- **Filmed screencasts**: they go stale with every release.

## Consequences

- A new screen gets its `?` by adding a help ID and passing `help`.
- Old releases' guides cost one mdBook build each; five is the cap.
- Tours 3 and 5 to 10 have scenes and captions; each is written once its
  screens exist. Sandboxes without Playwright's video encoder can run the
  tours but not record them.
- `vellum-render`'s stylesheet no longer lets a facts list, a chart's
  data or a calculator widen a page on a phone.
