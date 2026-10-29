---
id: 01M63GQREA4APWV4YR4E8PY0TQ
title: "ADR 0031: UI quality suite for layout, loading and visual regressions"
status: draft
owner: "@cruisinme30"
tags: [adr, web, testing, accessibility]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D123, D124, D125
- Issue: #222
- Pull requests: #313, #288, #289

:::

## Context

Every Vellum surface a person looks at (the web app and PWA, the desktop
shell, rendered pages, exports and the published website) has to render
correctly on phones and wide screens, in five themes, at 200 % zoom, in
right-to-left languages and with long text. axe alone finds accessibility
rules, not overlapping buttons, text spilling out of a card, a page that
scrolls sideways or a white screen while the bundle loads. The product
owner asked for the UI to be red-teamed and kept fixed as features land
(#222).

## Decisions

### `D123`: one layout audit, run on every page in every mode

- `web/e2e/ui/audit.ts` runs in the page and reports: horizontal overflow,
  elements outside the viewport, overlapping text or controls, clipped
  content, text wider than its box, images, icons and fonts that failed.
- The harness adds console errors, uncaught exceptions, failed requests,
  layout shift above 0.1 (Chromium) and axe WCAG 2.2 AA.
- A keyboard walk checks that each focused control changes pixels and is
  not covered; loading checks cover a slow network, no JavaScript, failed
  scripts and offline once a service worker controls the page.
- Intended overlaps and clips opt out in markup (`data-allow-overlap`,
  `data-allow-clip`), next to the design that needs them.
- Why: one rule set for every surface, with each failure naming the element.

### `D124`: the matrix and the theme contract

- Browsers: Chromium, Firefox, WebKit. Viewports: phone 390, tablet 820,
  laptop 1366, wide 2560 pixels. `UI_BROWSERS` and `UI_VIEWPORTS` narrow it.
- Modes: light, dark, the five app themes, forced colours, zoom 150, 200
  and 400 %, text at 200 %, WCAG text spacing with a dyslexia font, RTL,
  reduced motion, and a stress mode that lengthens every text with long
  words, CJK, Arabic, emoji and an unbroken URL.
- Themes are selected by `<html data-theme="light|dark|high-contrast|sepia|dim">`;
  the app must read that attribute at start so the suite can set it.
- Targets: app routes listed in `targets.ts`, plus every HTML page of the
  built website and help books, found on disk.
- Fast suite: Chromium at phone and laptop on a quick subset. Full suite:
  every browser, viewport and page; help pages that share a template get
  the default mode only ([ADR 0035](0035-ci-speed.md), D165).

### `D125`: visual baselines only for stable screens, in a pinned container

- Pixel baselines live in `web/e2e/__screenshots__/` and compare at 0.1 %
  of pixels, the brief's threshold.
- They are made and checked only inside the Playwright container
  (`mcr.microsoft.com/playwright`, same version as the package), because
  fonts and browser builds change pixels between machines.
- Only stable screens get baselines: the app shell now, then the golden
  corpus of blocks and Mermaid types and their exports as the renderer
  and exporters land. Pages whose words change weekly rely on the audit.
- `scripts/ci/ui-baselines.sh` refreshes the baselines in Docker.

## Alternatives

- **Visual diffs of every page**: fail on every copy edit; the audit finds the real bugs.
- **A hosted visual service (Percy, Chromatic)**: a third party sees unreleased UI, and costs money.
- **axe only**: misses overlap, overflow, loading and focus problems.
- **Chromium only**: WebKit and Firefox differ in flex, grid and font metrics.

## Consequences

- A new view is covered by adding its route to `targets.ts`; a new help
  book page is covered automatically.
- Layout bugs fail the fast suite before they merge, with a screenshot.
- The full matrix runs on release pull requests into main, in about 10 minutes.
- Revisit when the renderer lands: add the golden corpus pages as targets
  with baselines, and PDF and Word exports rendered to images.
