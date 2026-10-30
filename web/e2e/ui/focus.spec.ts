// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";

import { expect, test } from "./fixtures";
import { settle, type Problem, describe as lines } from "./harness";
import { fakeServer } from "./server";
import { TARGETS } from "./targets";

/**
 * Keyboard focus (WCAG 2.2 SC 2.4.7, 2.4.11): tabbing reaches each
 * control, the focus indicator changes pixels, and the focused control is
 * not hidden under something else (ADR 0031, D123).
 */
const MAX_STOPS = 30;
const PAD = 4;

interface Stop {
  path: string;
  x: number;
  y: number;
  width: number;
  height: number;
  obscured: boolean;
}

/** Waits out smooth scrolling: three frames without movement, at most 1 s. */
const scrollSettled = (page: Page): Promise<void> =>
  page.evaluate(async () => {
    const deadline = performance.now() + 1000;
    for (
      let still = 0, last = Number.NaN;
      still < 3 && performance.now() < deadline;
    ) {
      await new Promise((resolve) => requestAnimationFrame(resolve));
      const position = scrollX * 100_000 + scrollY;
      still = position === last ? still + 1 : 0;
      last = position;
    }
  });

const focusedStop = async (page: Page): Promise<Stop | undefined> => {
  await scrollSettled(page);
  return page.evaluate(() => {
    const element = document.activeElement;
    if (
      element === null ||
      element === document.body ||
      element === document.documentElement
    )
      return;
    const parts: string[] = [];
    for (
      let node: Element | null = element;
      node !== null && parts.length < 3;
      node = node.parentElement
    ) {
      parts.unshift(
        node.id === ""
          ? node.tagName.toLowerCase()
          : `${node.tagName.toLowerCase()}#${node.id}`,
      );
    }
    const rect = element.getBoundingClientRect();
    const top = document.elementFromPoint(
      rect.left + rect.width / 2,
      rect.top + rect.height / 2,
    );
    const obscured =
      top !== null && !element.contains(top) && !top.contains(element);
    const text = element.textContent.trim().slice(0, 30);
    return {
      path: `${parts.join(" > ")} "${text}"`,
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
      obscured,
    };
  });
};

async function indicatorVisible(page: Page, stop: Stop): Promise<boolean> {
  const size = page.viewportSize() ?? { width: 0, height: 0 };
  const x = Math.max(0, stop.x - PAD);
  const y = Math.max(0, stop.y - PAD);
  const clip = {
    x,
    y,
    width: Math.max(1, Math.min(size.width - x, stop.width + 2 * PAD)),
    height: Math.max(1, Math.min(size.height - y, stop.height + 2 * PAD)),
  };
  const focused = await page.screenshot({
    clip,
    animations: "disabled",
    caret: "hide",
  });
  await page.evaluate(() => {
    const element = document.activeElement;
    (globalThis as unknown as { vellumFocus?: Element | null }).vellumFocus =
      element;
    if (element instanceof HTMLElement || element instanceof SVGElement)
      element.blur();
    // Chromium with touch keeps a date input focused through blur(); park
    // focus on an invisible probe instead, so the "blurred" shot is unfocused.
    if (element !== null && document.activeElement === element) {
      const probe = document.createElement("span");
      probe.tabIndex = -1;
      probe.style.cssText =
        "position:fixed;inset:0 auto auto 0;width:1px;height:1px;opacity:0";
      probe.dataset["vellumProbe"] = "";
      document.body.append(probe);
      probe.focus({ preventScroll: true });
    }
  });
  const blurred = await page.screenshot({
    clip,
    animations: "disabled",
    caret: "hide",
  });
  await page.evaluate(() => {
    for (const probe of document.querySelectorAll("[data-vellum-probe]"))
      probe.remove();
    const element = (globalThis as unknown as { vellumFocus?: Element | null })
      .vellumFocus;
    if (element instanceof HTMLElement || element instanceof SVGElement)
      element.focus({ preventScroll: true });
  });
  return !focused.equals(blurred);
}

// Sweep pages share the template of the pages checked here (D165).
for (const target of TARGETS.filter((candidate) => !candidate.sweep)) {
  test(`${target.name} keyboard focus`, async ({
    page,
    browserName,
    isMobile,
  }, testInfo) => {
    // WebKit skips links on Tab unless the OS setting is on; phones have no Tab
    // key.
    test.skip(
      browserName === "webkit" || isMobile,
      "no Tab navigation to test",
    );
    await fakeServer(page, target.server);
    await page.goto(target.url);
    await settle(page);
    const problems: Problem[] = [];
    const seen = new Set<string>();
    for (let index = 0; index < MAX_STOPS; index += 1) {
      await page.keyboard.press("Tab");
      const stop = await focusedStop(page);
      if (stop === undefined || seen.has(stop.path)) break;
      seen.add(stop.path);
      const size = page.viewportSize() ?? { width: 0, height: 0 };
      if (
        stop.x + stop.width <= 0 ||
        stop.y + stop.height <= 0 ||
        stop.x >= size.width ||
        stop.y >= size.height
      ) {
        problems.push({
          rule: "focus-offscreen",
          target: stop.path,
          message: `focused at x=${stop.x.toFixed(0)} y=${stop.y.toFixed(0)}, out of view`,
        });
        continue;
      }
      if (stop.obscured)
        problems.push({
          rule: "focus-obscured",
          target: stop.path,
          message: "covered by another element",
        });
      if (!(await indicatorVisible(page, stop))) {
        problems.push({
          rule: "focus-not-visible",
          target: stop.path,
          message: "no visible change when focused",
        });
      }
    }
    await testInfo.attach("focus-order", {
      body: [...seen].join("\n"),
      contentType: "text/plain",
    });
    expect(lines(problems)).toEqual([]);
  });
}
