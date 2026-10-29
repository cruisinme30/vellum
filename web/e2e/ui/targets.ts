// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { readdirSync, statSync } from "node:fs";
import path from "node:path";

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
  options: { server?: ServerState; visual?: boolean } = {},
): Target => ({
  name: `app ${name}`,
  url: `http://127.0.0.1:${String(APP_PORT)}${route}`,
  app: true,
  fast: true,
  sweep: false,
  visual: options.visual ?? true,
  ...(options.server === undefined ? {} : { server: options.server }),
});

// Baselines are made in the Playwright container (scripts/ci/ui-baselines.sh).
const APP_TARGETS: Target[] = [
  app("shell", "/"),
  app("sign-in", "/sign-in", { server: "signed-out" }),
  app("setup", "/setup", { server: "new" }),
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
