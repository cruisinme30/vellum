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
- Decisions: D109, D110, D111, D307, D308
- Issue: #279 (redesign), #549 (type and theme toggle); the first version predates the issue rule
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
- No JavaScript is needed (D308 adds one small script, for the theme
  toggle): the product tabs are radio buttons styled with
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

### D307: book type in mdBook's 10-pixel rems, fonts that always arrive

- mdBook sets the root font size to 62.5%, so a rem inside a book is
  10 px, not 16. `site/book.css` sizes everything in those rems: body
  text 17 px (`1.7rem`), the site bar and feedback line 14 px. The first
  version used 16-pixel rems and printed body text at 10.6 px (#549).
- Fonts use `font-display: swap` and the Google Sans Flex file is
  preloaded on the docs home and every book page (`site/book-head.hbs`),
  so readers always get the site font. `optional` kept the fallback for
  the whole visit whenever the font took over ~100 ms, which over the
  network was most first visits. The UI suite's layout-shift gate still
  passes on a slow network.
- The build fills in the font path per book through a placeholder, so the older
  releases' books at `docs/v/<tag>/` find the fonts too, and it strips
  mdBook's own font stylesheet from every page, not only the top level.
- Smaller fixes: chapter numbers no longer run into titles, the book
  title in the menu bar is weight 500 (not 200), and the site bar sits
  above mdBook's full-height next-chapter arrow that hid its end.

### D308: one light and dark toggle for the whole site

- Every page (landing page, docs home, every book page) has a toggle at
  the end of its top bar. Until the reader presses it, the site follows
  the system setting; the choice is then kept in this browser
  (`localStorage`, key `vellum-site-theme`) for the whole site. Nothing
  is sent anywhere.
- `site/theme.js` is the one script the landing page and docs home run.
  Their policy allows scripts from the site itself only (`script-src
  'self'`); inline scripts stay blocked, and `check-site.sh` fails on any
  script other than `theme.js`. It loads in `<head>`, so the theme is set
  before first paint, and the toggle is hidden when JavaScript is off.
- In the books, "light" and "dark" are mdBook's recoloured light and navy
  themes; mdBook's menu of five themes is hidden, since three were never
  recoloured.

## Alternatives

- **A site generator with a theme (Docusaurus, Astro)**: more tooling,
  JavaScript by default, and a second copy of the docs to maintain.
- **Fonts from Google's servers**: simpler, but every visit would tell a
  third party who is reading.
- **An animated product tour in JavaScript**: livelier, but it breaks the
  no-script rule; a short video does the same job.
- **Deploy from `develop`**: the site would show unreleased work.
- **A toggle without JavaScript** (a checkbox and `:has()`): it cannot
  remember the choice from one page to the next.
- **Resetting mdBook's root size to 100%**: mdBook's own layout is sized
  in 10-pixel rems and would grow by 60%.

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
