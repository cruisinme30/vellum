---
id: 01M63BBFTTVJ9X5ZN77W65W1AE
title: "ADR 0027: Project website on GitHub Pages"
status: verified
owner: "@cruisinme30"
tags: [adr, docs, site]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D109, D110, D111
- Issue: none (recorded before the issue rule, D119)
- Pull requests: #31

:::

## Context

Vellum needs a public home: what it is, how to install it, the licence,
and the three help books (brief section 12, "published with mdBook"). The
project has no budget for hosting, and the repository is public.

## Decisions

### D109: GitHub Pages at the repository address

- The site is served by GitHub Pages at
  `https://cruisinme30.github.io/vellum/`, which is free for public
  repositories.
- `vellum.github.io` is not available: the `vellum` account on GitHub
  belongs to someone else. A free organisation under another name, or a
  custom domain, can be added later.
- Every link inside the site is relative, so moving to a custom domain or
  to an organisation needs only a `CNAME` setting, no rework.

### D110: a static page with no scripts, trackers or external requests

- The landing page is hand-written HTML and one stylesheet in `site/`,
  with Material 3 colours from the Vellum seed (brief section 13) in light
  and dark schemes. It has no JavaScript, no web fonts and no analytics;
  Roboto is used only when the visitor has it installed.
- A Content Security Policy in the page allows only its own stylesheet and
  images, so nothing can load from another origin.
- The documentation cards on the landing page are generated at build time
  from the books that exist, so the page never links to a missing book.

### D111: built from the repository with pinned mdBook, published from main

- `scripts/ci/build-site.sh` builds the landing page, every help book
  `docs/<name>.book/` at `docs/<name>/`, and the decision records in
  `docs/adr/` as one more book at `docs/decisions/`.
- Books are written in Vellum Markdown. The build drops front matter (its
  title becomes the page heading when the page has none) and unwraps
  `:::` containers, leaving code fences untouched, so unmodified mdBook
  renders them. Diagrams and charts show as their source until the
  renderer can publish them.
- mdBook is pinned (0.5.4) and its download checked against a SHA-256.
- The `Website` workflow builds the site on every pull request that
  touches it, and deploys it with `actions/deploy-pages` on every push to
  `main`. The site therefore always matches the last release train, not
  work in progress on `develop`.

## Alternatives

- **Paid hosting or a custom domain now**: no budget, and relative
  links make the move cheap later.
- **A static site generator with a theme**: more tooling and scripts
  for one page.
- **Deploy from `develop`**: the site would show unreleased work.

## Consequences

- The site always matches the last release, not work in progress.
- One-time setup by the repository owner: Settings, Pages, Source set
  to "GitHub Actions", and `main` allowed to deploy to `github-pages`.
