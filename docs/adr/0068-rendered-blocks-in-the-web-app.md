---
id: 01M651VHYTTB47V3GBBVB85NHM
title: "ADR 0068: Server-rendered blocks in the web app"
status: verified
owner: "@cruisinme30"
tags: [adr, render, editor, web, observability]
vellum: 1
---

::: facts

- Date: 2026-10-30
- Status: accepted
- Decisions: D286, D287, D288, D289, D302
- Issue: #522, #537
- Pull requests: to be linked when merged

:::

## Context

The server draws every block (Mermaid, Kroki diagrams, charts, timelines,
roadmaps, risk matrices, architecture, boards, calculators) to sanitised,
themed HTML (ADR 0007), and `GET /v1/pages/{id}/render` serves whole pages
with a cache and `304` answers (ADR 0061). The web app never asked for
it: the editor drew only a rough chart and CSV tables, and showed every
other block as source with "drawn on save"
([#522](https://github.com/cruisinme30/vellum/issues/522)). R-ED, R-RN
and R-HG-2 want every block shown drawn, R-TH-3 wants figures to follow
the app's theme, and R-OB-1 wants a "Copy diagnostic" on errors.

Two constraints shaped the design. The editor holds a block, not a saved
page, so a page render cannot draw what is being typed. And the app's
Content Security Policy has `style-src 'self'`: the renderer's inline
`style` attributes and Mermaid's `<style>` would be dropped if the HTML
were inserted as markup.

## Decisions

### D286: `render.block` draws one fence for the editor

- `POST /v1/render/block` takes `lang`, `args` and `source` and answers
  the same sanitised HTML, warnings and stale flag a page render gives
  that block. It is a POST only because the body can be long; it changes
  nothing.
- The server writes the fence itself, longer than any backtick run in the
  body, so the body cannot close it and smuggle in other blocks. `lang`
  is a fence word, `args` one line, the body at most 64 KiB.
- It resolves nothing (`NoResolver`): no wikilinks, no live issues. The
  answer depends only on the block, so any signed-in reader may ask.

### D287: one theme stylesheet for rendered HTML, scoped to the app

- `GET /v1/render/stylesheet` serves the renderer's themes and component
  styles nested under `.vl-render`, with each built-in theme keyed off
  the app's `<html data-theme>`. A theme change restyles every figure in
  place, with no new request (R-TH-3). It revalidates like page renders.
- The app adds it as a constructed stylesheet, which the policy allows.
  Rendered HTML is parsed in a detached document; `style` attributes are
  re-applied through the CSS object model and `<style>` text goes into a
  constructed stylesheet. No policy change, no inline style allowance.
- Figures keep the renderer's contrast-checked colours rather than mixing
  in the app's own tokens, which broke text and background pairs.

### D288: the editor shows the server's drawing, source on demand

- With a server, each data fence asks `render.block`: at once on open,
  then 400 ms after typing pauses, dropping answers for older text. The
  client caches 200 drawings per tab.
- A Source button shows or hides the editable source; it starts open
  only for an empty block. Warnings show under the drawing.
- When the request fails, the block says why, with Copy diagnostic, and
  falls back to the light client preview. Collections stay client-side.
- Calculator sliders recompute in the browser with the renderer's
  arithmetic and number format, ported to TypeScript.

### D289: a reading view, and Copy diagnostic on every error

- With a server, a page has a Read / Edit switch; `?view=read` shows the
  page from `render.page`. The editor stays the default, since it now
  draws blocks itself; demo mode has no switch.
- Failed requests carry their route and time as well as the request ID.
  Every error message built from them, and the shared loading frame, has
  a Copy diagnostic button: message, request ID, route, time, screen.
  It holds no page content or credentials. A blocked clipboard shows the
  text to copy by hand.

### D302: figures follow the accent colour

- `render.stylesheet` takes an optional `seed` (`rrggbb`). With it, every
  built-in theme in the sheet is `Theme::from_seed(theme, seed)`, so
  figures keep contrast-checked pairs while primary, chart, callout and
  syntax colours lean towards the accent. A bad seed is a 400.
- The app asks with the accent in use: a custom theme's seed, else the
  user's, else the organisation's
  ([#537](https://github.com/cruisinme30/vellum/issues/537)). Vellum's own
  seed sends none, so most users share one cached sheet. A new accent
  swaps the sheet in place; only the newest request's answer is adopted.
- A custom theme's token overrides still do not reach figures: they are
  checked against the app's pairs, not the renderer's (see Alternatives).

## Alternatives

- **Draw blocks from the page render, by block ID**: shows the saved page,
  not what is being typed, and re-renders the whole page per keystroke.
- **Draw in the browser** (Mermaid.js and friends): a second renderer to
  keep in step with the server, megabytes of script, and no Kroki.
- **Allow inline styles in the policy**: weakens it for the whole app to
  avoid a few lines of DOM work.
- **Bridge the app's colour tokens into figures**: tried; custom pairs
  failed WCAG contrast in High contrast. Seed colours can come later from
  `Theme::from_seed` on the server.
- **Reading view as the default**: changes how every page opens, for a
  view the editor now mostly covers.

## Consequences

- The UI suite (`web/e2e/render.spec.ts`) checks every block drawn in all
  five themes, with no overflow and axe clean, plus the editor, a theme
  change and Copy diagnostic. It replays `render-fixture.gen.json`, which
  a `vellum-cli` test regenerates from the real renderer.
- Those tests found Mermaid edge labels unreadable in High contrast:
  label text on `surface-container`. Edge labels now sit on
  `primary-container`, with a contrast test over every theme;
  `RENDER_VERSION` is 3, so cached renders refresh.
- The accent seed and a custom theme's seed tint figures (D302); a
  custom theme's individual token overrides do not.
- `web/e2e/render.spec.ts` passes in Chromium, Firefox and WebKit at
  laptop size, including the accent test.
