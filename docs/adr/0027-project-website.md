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
- Issue: #279 (redesign); the first version predates the issue rule
- Pull requests: #31, #325

:::

## Context

Vellum needs a public home that explains what it is and why it is worth
anyone's time, and that holds all of its documentation. The project has no
budget for hosting, and the repository is public.

The first site (pull request #31) was a single text page that read like
documentation and sent readers to GitHub. Issue #279 asked for a product
site: a clear promise, real screens, a video, a comparison with other
tools, Google's colours, and every guide readable on the site itself.

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

### D110: a product page with no scripts, trackers or outside requests

- The landing page in `site/` sells the product first: the promise, a
  27-second tour video, six product areas with screenshots, a comparison
  table, setup steps and links into the guides. A banner says plainly
  that 1.0 is not released and the screens show what it will ship.
- It uses Google's Material 3 colours and the Google Sans Flex and Google
  Sans Code fonts (SIL Open Font License), served from the site itself,
  in light and dark schemes that follow the visitor's system.
- There is no JavaScript: the product tabs are radio buttons styled with
  CSS. The tour video has controls, a text transcript, and does not play
  by itself when the visitor asks for reduced motion.
- A Content Security Policy allows only the site's own styles, images,
  fonts and video, so nothing loads from another server and no cookies
  are set.
- The layout survives right-to-left text, 200% text size and a 390-pixel
  phone without sideways scrolling (issue #248).

### D111: built from the repository with pinned mdBook, checked, published from main

- `scripts/ci/build-site.sh` builds the landing page, every help book
  `docs/<name>.book/` at `docs/<name>/`, the decision records as one more
  book at `docs/decisions/`, and a docs home at `docs/` that lists every
  chapter of every guide, read from each book's `SUMMARY.md`.
- The books are restyled to match the site and carry a bar linking back
  to the site, the docs home and the other guides, so a reader never needs
  GitHub to find documentation. Counts on the pages (help pages, crates,
  decisions) are computed at build time, so they never go stale.
- Vellum Markdown becomes plain CommonMark on the way: front matter is
  dropped and `:::` containers unwrapped, so unmodified mdBook renders it.
  Links are rewritten for the published layout.
- `scripts/ci/check-site.sh` then fails the build if a page lacks the
  security policy, runs a script, uses inline styles, has an image without
  alt text, or links to a file that does not exist.
- mdBook is pinned (0.5.4) and its download checked against a SHA-256.
  The `Website` workflow builds and checks the site on every pull request
  that touches it, and deploys it on every push to `main`, so the site
  matches the last release, not work in progress.

## Alternatives

- **A site generator with a theme (Docusaurus, Astro)**: more tooling,
  JavaScript by default, and a second copy of the docs to maintain.
- **Fonts from Google's servers**: simpler, but every visit would tell a
  third party who is reading.
- **An animated product tour in JavaScript**: livelier, but it breaks the
  no-script rule; a short video does the same job.
- **Deploy from `develop`**: the site would show unreleased work.

## Consequences

- The screenshots and video are rendered from a design of the Northstar
  demo, not the shipping app. They are re-recorded from the real app
  before 1.0 (follow-up on issue #279).
- The comparison table is a claim about other products and needs a
  check each release; readers are invited to report mistakes.
- Manual pages for the command line are generated from the CLI
  definitions in a follow-up; until then the command reference page in
  the admin guide is the manual.
- One-time setup by the repository owner: Settings, Pages, Source set
  to "GitHub Actions", and `main` allowed to deploy to `github-pages`.
