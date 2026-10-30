// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";

/**
 * The display modes every page is checked in (ADR 0031, D124). A mode
 * changes how the page is shown, never what it says, except `stress`,
 * which lengthens every text to find layouts that only fit short copy.
 */
export interface Mode {
  name: string;
  /** Applies the mode before the page is opened. */
  apply: (page: Page) => Promise<void>;
  /** Only for targets that follow the app's theme contract. */
  appOnly?: boolean;
  /** Only in Chromium (CDP or media features the others lack). */
  chromiumOnly?: boolean;
  /** Skip axe: the mode is a synthetic stress, not a real reading setting. */
  skipAxe?: boolean;
}

const initScript = async (page: Page, script: string): Promise<void> => {
  await page.addInitScript({ content: script });
};

/**
 * Runs `body` as soon as the parser creates <html>, before first paint
 * and before the page's own scripts. (An init script runs before the
 * document has any elements.)
 */
const onDocument = (body: string): string =>
  `(() => {
    const run = () => { ${body} };
    if (document.documentElement) { run(); return; }
    const observer = new MutationObserver(() => {
      if (document.documentElement) { observer.disconnect(); run(); }
    });
    observer.observe(document, { childList: true });
  })();`;

// A constructed style sheet: CSSOM, so a strict style-src CSP does not block
// it.
const addStyle = (css: string): string =>
  onDocument(
    `const sheet = new CSSStyleSheet(); sheet.replaceSync(${JSON.stringify(css)}); document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];`,
  );

/**
 * The theme contract: `<html data-theme="…">`, read by the app at start (D124).
 */
const theme = (name: string, scheme: "light" | "dark"): Mode => ({
  name: `theme-${name}`,
  appOnly: true,
  apply: async (page) => {
    await page.emulateMedia({ colorScheme: scheme });
    await initScript(
      page,
      onDocument(
        `document.documentElement.dataset.theme = ${JSON.stringify(name)};`,
      ),
    );
  },
});

/**
 * Browser zoom: the same CSS layout as a viewport divided by the zoom factor.
 */
const zoom = (percent: number): Mode => ({
  name: `zoom-${String(percent)}`,
  apply: async (page) => {
    const size = page.viewportSize();
    if (size !== null) {
      await page.setViewportSize({
        width: Math.max(320, Math.round((size.width * 100) / percent)),
        height: Math.round((size.height * 100) / percent),
      });
    }
  },
});

// Long words, combining marks, CJK, Arabic, emoji with skin tones and ZWJ, and
// an unbroken URL.
const STRESS =
  " Ünïcödé façade 中文字符测试 العربية עברית 🧑🏽‍💻👩‍👩‍👧 " +
  "Donaudampfschifffahrtsgesellschaftskapitän " +
  "https://northstar.dev/a/very/long/path/without/any/break/points/at/all";

const ALL_MODES: readonly Mode[] = [
  {
    name: "light",
    apply: (page) => page.emulateMedia({ colorScheme: "light" }),
  },
  { name: "dark", apply: (page) => page.emulateMedia({ colorScheme: "dark" }) },
  theme("light", "light"),
  theme("dark", "dark"),
  theme("high-contrast", "light"),
  theme("sepia", "light"),
  theme("dim", "dark"),
  {
    name: "forced-colors",
    chromiumOnly: true,
    apply: (page) =>
      page.emulateMedia({ forcedColors: "active", contrast: "more" }),
  },
  zoom(150),
  zoom(200),
  zoom(400),
  {
    name: "text-200",
    // Text-only zoom: the root font size doubles, boxes sized in px do not.
    apply: (page) =>
      initScript(page, addStyle("html { font-size: 200% !important; }")),
  },
  {
    name: "text-spacing",
    // WCAG 2.2 SC 1.4.12, plus a wide dyslexia-friendly font stack.
    apply: (page) =>
      initScript(
        page,
        addStyle(
          "* { line-height: 1.5 !important; letter-spacing: 0.12em " +
            "!important; word-spacing: 0.16em !important; font-family: " +
            "OpenDyslexic, 'Comic Sans MS', Verdana, sans-serif " +
            "!important; } p { margin-bottom: 2em !important; }",
        ),
      ),
  },
  {
    name: "rtl",
    apply: (page) =>
      initScript(
        page,
        onDocument(
          `document.documentElement.dir = "rtl"; ` +
            `document.documentElement.lang = "ar";`,
        ),
      ),
  },
  {
    name: "reduced-motion",
    apply: (page) => page.emulateMedia({ reducedMotion: "reduce" }),
  },
  {
    name: "stress",
    skipAxe: true,
    // Lengthens text as the parser adds it, so the page never paints the short
    // copy.
    apply: (page) =>
      initScript(
        page,
        `(() => {
          const extra = ${JSON.stringify(STRESS)};
          const stretch = (node) => {
            const parent = node.parentElement;
            if (!parent || /^(SCRIPT|STYLE|CODE|PRE|KBD|TITLE)$/.test(parent.tagName)) return;
            if ((node.textContent ?? "").trim().length > 1 && !node.textContent.endsWith(extra)) node.textContent += extra;
          };
          new MutationObserver((records) => {
            for (const record of records) {
              for (const added of record.addedNodes) {
                if (added.nodeType === Node.TEXT_NODE) { stretch(added); continue; }
                if (added.nodeType !== Node.ELEMENT_NODE) continue;
                const walker = document.createTreeWalker(added, NodeFilter.SHOW_TEXT);
                for (let node = walker.nextNode(); node; node = walker.nextNode()) stretch(node);
              }
            }
          }).observe(document, { childList: true, subtree: true });
        })();`,
      ),
  },
];

/**
 * The modes in scope: all of them, or the comma-separated names in
 * UI_MODES (for example the five themes, for a theme sweep).
 */
const wanted = process.env["UI_MODES"]?.split(",").map((name) => name.trim());
export const MODES: readonly Mode[] =
  wanted === undefined
    ? ALL_MODES
    : ALL_MODES.filter((mode) => wanted.includes(mode.name));
