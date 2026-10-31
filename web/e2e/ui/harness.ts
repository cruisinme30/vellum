// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { writeFile } from "node:fs/promises";

import { AxeBuilder } from "@axe-core/playwright";
import type { Page, TestInfo } from "@playwright/test";

import { layoutAudit, type Finding } from "./audit";
import type { Mode } from "./modes";
import { fakeServer } from "./server";
import type { Target } from "./targets";

/** WCAG 2.2 AA, the brief's accessibility bar. */
const AXE_TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
/** Cumulative layout shift above this fails ("good" in Core Web Vitals). */
export const CLS_LIMIT = 0.1;

export interface Problem {
  rule: string;
  target: string;
  message: string;
}

/** Records console errors, uncaught exceptions and failed requests. */
function watchErrors(page: Page): Problem[] {
  const problems: Problem[] = [];
  page.on("console", (message) => {
    if (message.type() === "error")
      problems.push({
        rule: "console-error",
        target: "console",
        message: message.text(),
      });
  });
  page.on("pageerror", (error) => {
    problems.push({
      rule: "uncaught-exception",
      target: "window",
      message: error.message,
    });
  });
  page.on("requestfailed", (request) => {
    // Browsers cancel a video source they won't play (WebKit and WebM) or
    // no longer need, WebKit cancels font subsets (symbols, math) once the
    // text turns out not to need them, and a screen cancels its own fetch
    // when it unmounts (D285). None of these is a failed load.
    const error = request.failure()?.errorText ?? "";
    const cancellable =
      ["media", "font", "fetch", "xhr"].includes(request.resourceType()) ||
      /\.(?:webm|mp4|ogg|mp3|woff2?)$/.test(request.url());
    if (cancellable && /abort|cancel/i.test(error)) return;
    problems.push({
      rule: "request-failed",
      target: request.url(),
      message: request.failure()?.errorText ?? "failed",
    });
  });
  page.on("response", (response) => {
    if (response.status() >= 400) {
      problems.push({
        rule: "http-error",
        target: response.url(),
        message: `HTTP ${String(response.status())}`,
      });
    }
  });
  return problems;
}

/**
 * Starts counting layout shifts before the page's first byte (Chromium only).
 */
export async function watchLayoutShift(page: Page): Promise<void> {
  await page.addInitScript({
    content: `window.__vellumCls = 0;
      window.__vellumShifts = [];
      try {
        new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            if (entry.hadRecentInput) continue;
            window.__vellumCls += entry.value;
            for (const source of entry.sources ?? []) {
              const node = source.node;
              const name = node && node.nodeType === 1
                ? node.tagName.toLowerCase() + (node.id ? "#" + node.id : "") +
                  (node.classList.length
                    ? "." + [...node.classList].join(".")
                    : "")
                : String(node && node.nodeName);
              const from = source.previousRect, to = source.currentRect;
              window.__vellumShifts.push(
                name + " moved " + Math.round(from.x) + "," +
                  Math.round(from.y) + " → " + Math.round(to.x) + "," +
                  Math.round(to.y) + " at " + Math.round(entry.startTime) +
                  " ms (" + entry.value.toFixed(3) + ")");
            }
          }
        }).observe({ type: "layout-shift", buffered: true });
      } catch {}`,
  });
}

export async function layoutShift(page: Page): Promise<number> {
  return page.evaluate(
    () => (globalThis as unknown as { __vellumCls?: number }).__vellumCls ?? 0,
  );
}

/**
 * Waits until the page is quiet: network idle, fonts loaded, two frames
 * painted.
 */
export async function settle(page: Page): Promise<void> {
  // Pages that poll never go idle, nor does WebKit while a looping video
  // streams, so the wait is capped; the fonts and frames below still settle
  // them.
  await page
    .waitForLoadState("networkidle", { timeout: 10_000 })
    .catch((error: unknown) => {
      if (!(error instanceof Error && error.name === "TimeoutError"))
        throw error;
    });
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(resolve)),
    );
  });
}

/**
 * Attaches a screenshot and the problem list as files, so reports link to
 * them instead of inlining megabytes of base64. Very long pages (a book's
 * print page) are shot at viewport height.
 */
async function attachEvidence(
  page: Page,
  testInfo: TestInfo,
  problems: Problem[],
): Promise<void> {
  const height = await page.evaluate(
    () => document.documentElement.scrollHeight,
  );
  const fullPage = height <= (page.viewportSize()?.height ?? 0) * 4;
  const screenshot = testInfo.outputPath("page.png");
  await page.screenshot({ path: screenshot, fullPage });
  await testInfo.attach("page", { path: screenshot, contentType: "image/png" });
  if (problems.length > 0) {
    const file = testInfo.outputPath("problems.json");
    await writeFile(file, JSON.stringify(problems, undefined, 2));
    await testInfo.attach("problems", {
      path: file,
      contentType: "application/json",
    });
  }
}

/**
 * Presses the target's `open` button, if any, and waits for what it opens.
 * The stress mode lengthens every text, so there the button's name only
 * has to start with the words.
 */
export async function openPanel(
  page: Page,
  target: Target,
  mode?: Mode,
): Promise<void> {
  if (target.open === undefined) return;
  const name =
    mode?.name === "stress"
      ? new RegExp(target.open.source.replace(/\$$/, ""), target.open.flags)
      : target.open;
  await page.getByRole("button", { name }).first().click();
  await settle(page);
  // A panel's pictures (an export preview) decode after the network is
  // idle; give the ones in view a moment so the audit does not race them.
  await page
    .waitForFunction(
      () =>
        [...document.images].every(
          (image) =>
            image.complete ||
            image.getBoundingClientRect().top > window.innerHeight,
        ),
      undefined,
      { timeout: 5000 },
    )
    .catch(() => {
      // The audit reports any picture still missing.
    });
}

/** Opens a target in a mode and returns everything wrong with it. */
export async function auditTarget(
  page: Page,
  target: Target,
  mode: Mode,
  testInfo: TestInfo,
): Promise<Problem[]> {
  const errors = watchErrors(page);
  await watchLayoutShift(page);
  await mode.apply(page);
  await fakeServer(page, target.server);
  await page.goto(target.url, { waitUntil: "load" });
  await settle(page);
  await openPanel(page, target, mode);

  const problems: Problem[] = [...errors];
  const findings: Finding[] = await page.evaluate(layoutAudit);
  problems.push(...findings);

  const cls = await layoutShift(page);
  if (cls > CLS_LIMIT) {
    const shifts = await page.evaluate(
      () =>
        (globalThis as unknown as { __vellumShifts?: string[] })
          .__vellumShifts ?? [],
    );
    problems.push({
      rule: "layout-shift",
      target: "page",
      message:
        `CLS ${cls.toFixed(3)} > ${String(CLS_LIMIT)}: ` +
        shifts.slice(0, 3).join("; "),
    });
  }

  if (mode.skipAxe !== true) {
    const axe = await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze();
    for (const violation of axe.violations) {
      for (const node of violation.nodes) {
        problems.push({
          rule: `axe:${violation.id}`,
          target: node.target.join(" "),
          message: violation.help,
        });
      }
    }
  }

  await attachEvidence(page, testInfo, problems);
  return problems;
}

/** One line per problem, for a readable assertion message. */
export const describe = (problems: Problem[]): string[] =>
  problems.map(
    (problem) => `${problem.rule} @ ${problem.target}: ${problem.message}`,
  );
