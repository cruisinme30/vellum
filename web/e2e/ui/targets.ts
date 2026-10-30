// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { readdirSync, statSync } from "node:fs";
import path from "node:path";

import { HANDBOOK, LAUNCH_PAGE, LAUNCH_PROJECT } from "./northstar-server";
import type { ServerState } from "./server";

/**
 * What the UI suite opens (ADR 0031, D123). App routes are listed by hand
 * as views land; the static site (landing page and help books) is found
 * on disk, so a new book page is covered the day it is written.
 */
export interface Target {
  name: string;
  url: string;
  /** Follows the app's theme contract (`data-theme`). */
  app: boolean;
  /** Part of the quick subset in the fast suite. */
  fast: boolean;
  /**
   * Checked in the default display mode only (D165). The help-book
   * chapters share one mdBook template, and each book's print page, which
   * gets every check, already lays out every chapter's content, so the
   * other modes would repeat those checks page by page.
   * UI_SCOPE=exhaustive checks these pages in every mode too.
   */
  sweep: boolean;
  /**
   * Has a visual baseline. Only stable screens qualify: a pixel baseline
   * of a page whose words change every week fails for no reason, and the
   * layout audit already covers those pages.
   */
  visual: boolean;
  /** What the stand-in server says; absent for the demo and the site. */
  server?: ServerState;
  /**
   * The name of a button to press once the page has settled, to audit
   * the dialog or panel it opens (D285) rather than the page under it.
   */
  open?: RegExp;
}

/** quick (the default): the fast subset; full: every page; exhaustive: every page in every mode. */
const scope = process.env["UI_SCOPE"] ?? "quick";

export const APP_PORT = 4173;
export const SITE_PORT = 4174;
/** The built site (scripts/ci/build-site.sh), or the bare landing page. */
export const SITE_DIR = path.resolve(
  import.meta.dirname,
  "../..",
  process.env["UI_SITE_DIR"] ?? "../site",
);

const app = (
  name: string,
  route: string,
  options: { server?: ServerState; visual?: boolean; fast?: boolean; open?: RegExp } = {},
): Target => ({
  name: `app ${name}`,
  url: `http://127.0.0.1:${String(APP_PORT)}${route}`,
  app: true,
  fast: options.fast ?? true,
  sweep: false,
  visual: options.visual ?? true,
  ...(options.server === undefined ? {} : { server: options.server }),
  ...(options.open === undefined ? {} : { open: options.open }),
});

/**
 * The other screens that need a server (D285), signed in to the Northstar
 * stand-in with a fixed clock. A few stand for them in the fast suite;
 * the full suite opens them all. Their figures and dates come from the
 * fixture, so they can have pixel baselines.
 */
const screen = (name: string, route: string, fast = false): Target =>
  app(name, route, { server: "signed-in", fast, visual: fast });

/** A dialog or panel on a server screen, opened by pressing `button`. */
const panel = (name: string, route: string, button: RegExp): Target =>
  app(name, route, { server: "signed-in", fast: false, visual: false, open: button });

function serverScreens(): Target[] {
  return [
    screen("home (server)", "/"),
    screen("my-work", "/my-work", true),
    screen("issues", "/issues", true),
    screen("issue", "/issues/ENG-12"),
    screen("books", "/books"),
    screen("book section", `/books/${HANDBOOK}/setup`, true),
    panel("book publish", `/books/${HANDBOOK}/setup`, /^Publish$/),
    panel("book snapshots", `/books/${HANDBOOK}/setup`, /^Snapshots$/),
    panel("book section access", `/books/${HANDBOOK}/setup`, /^Section access$/),
    panel("book outline history", `/books/${HANDBOOK}/setup`, /^Outline history$/),
    panel("book move section", `/books/${HANDBOOK}/setup`, /^Move to another book$/),
    screen("appearance (server)", "/settings/appearance"),
    screen("notification settings", "/settings/notifications"),
    screen("private spaces", "/settings/private-spaces"),
    screen("admin", "/admin"),
    screen("admin usage", "/admin/usage", true),
    screen("admin requests", "/admin/requests"),
    screen("admin routes", "/admin/routes"),
    screen("admin audit", "/admin/audit"),
    screen("admin settings", "/admin/settings"),
    screen("admin backups", "/admin/backups"),
    screen("admin webhooks", "/admin/webhooks"),
    screen("admin users", "/admin/users"),
    screen("admin spaces", "/admin/spaces"),
    screen("admin groups", "/admin/groups"),
    screen("admin tokens", "/admin/tokens"),
    screen("admin sign-in", "/admin/sign-in"),
    screen("api tokens", "/settings/api-tokens"),
    screen("security settings", "/settings/security"),
    screen("help", "/help"),
    screen("collections (server)", "/data"),
    // Planning (#543): projects, initiatives, insights, intake forms.
    screen("projects", "/projects"),
    screen("project", `/projects/${LAUNCH_PROJECT}`),
    screen("initiatives", "/initiatives"),
    screen("insights", "/insights"),
    screen("asks", "/asks"),
    screen("ask form", "/asks/ENG/bug-report"),
    screen("new issue", "/issues/new"),
    // AI and MCP (#543): bring-your-own-model settings, Ask, MCP setup.
    screen("ai settings", "/settings/ai"),
    screen("ask", "/ask"),
    screen("admin mcp", "/admin/mcp"),
    // Import, and a page's share, export and assistant panels (#543).
    screen("import", "/import"),
    screen("page (server)", `/p/${LAUNCH_PAGE}`),
    panel("page share", `/p/${LAUNCH_PAGE}`, /^Share$/),
    panel("page export", `/p/${LAUNCH_PAGE}`, /^Export$/),
    panel("page assistant", `/p/${LAUNCH_PAGE}`, /^Assistant$/),
  ];
}

// Baselines are made in the Playwright container (scripts/ci/ui-baselines.sh).
const APP_TARGETS: Target[] = [
  app("shell", "/"),
  app("sign-in", "/sign-in", { server: "signed-out" }),
  app("setup", "/setup", { server: "new" }),
  // The Drive (ADR 0052) against a stand-in drive; its dates and sizes
  // follow the browser's locale, so no pixel baselines.
  app("drive", "/drive", { server: "drive", visual: false }),
  app("drive folder", "/drive/s/S1/f/F1", { server: "drive", visual: false }),
  app("drive trash", "/drive/s/S1/trash", { server: "drive", visual: false }),
  app("drive settings", "/drive/s/S2/settings", { server: "drive", visual: false }),
  app("drive search", "/drive/search?q=plan&kind=page", { server: "drive", visual: false }),
  app("drive recent", "/drive/recent", { server: "drive", visual: false, fast: false }),
  app("drive starred", "/drive/starred", { server: "drive", visual: false, fast: false }),
  app("drive shared drives", "/drive/shared", { server: "drive", visual: false, fast: false }),
  app("drive shared with me", "/drive/shared-with-me", {
    server: "drive",
    visual: false,
    fast: false,
  }),
  // The theme editor's words and its contrast table change as tokens are
  // added, so the layout audit covers it rather than a pixel baseline.
  app("appearance", "/settings/appearance", { visual: false }),
  app("issues board", "/issues?layout=board", { server: "work", visual: false }),
  app("issues list", "/issues?group=assignee", { server: "work", visual: false }),
  app("my issues", "/issues/mine", { server: "work", visual: false }),
  app("triage", "/issues/triage", { server: "work", visual: false }),
  app("cycles", "/issues/cycles", { server: "work", visual: false }),
  app("team settings", "/issues/settings", { server: "work", visual: false }),
  app("templates", "/issues/templates", { server: "work", visual: false }),
  ...serverScreens(),
];

function htmlFiles(directory: string): string[] {
  let entries: string[];
  try {
    entries = readdirSync(directory);
  } catch {
    return [];
  }
  return entries.flatMap((entry) => {
    const full = path.join(directory, entry);
    if (statSync(full).isDirectory()) return htmlFiles(full);
    return entry.endsWith(".html") ? [full] : [];
  });
}

// mdBook's own redirect stubs and the 404 page are not content.
const SKIP = new Set(["404.html", "toc.html"]);

function siteTargets(): Target[] {
  const pages = htmlFiles(SITE_DIR)
    .map((file) => path.relative(SITE_DIR, file).split(path.sep).join("/"))
    .filter((relative) => !SKIP.has(path.posix.basename(relative)))
    .toSorted((a, b) => a.localeCompare(b));
  // The fast subset, and the pages every check runs on in the full suite:
  // the landing page, the docs home, each book's first page and its print page.
  const fast = new Set(["index.html", "docs/index.html"]);
  for (const page of pages) {
    const book = /^docs\/[^/]+\//.exec(page)?.[0];
    if (book !== undefined) {
      fast.add(`${book}index.html`);
      fast.add(`${book}print.html`);
    }
  }
  return pages.map((page) => ({
    name: `site ${page}`,
    url: `http://127.0.0.1:${String(SITE_PORT)}/${page}`,
    app: false,
    fast: fast.has(page),
    sweep: !fast.has(page) && scope !== "exhaustive",
    visual: false,
  }));
}

const quick = scope !== "full" && scope !== "exhaustive";

/** Every target in scope: the fast subset unless UI_SCOPE is full or exhaustive. */
export const TARGETS: readonly Target[] = [...APP_TARGETS, ...siteTargets()].filter(
  (target) => !quick || target.fast,
);
