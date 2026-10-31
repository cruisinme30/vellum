// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The help videos (R-HG-5, ADR 0024 D258) are recorded, not filmed: each
 * tour drives the app against the Northstar demo workspace, Playwright
 * records the screen, and every scene's caption becomes a WebVTT cue. A
 * release re-records them with `pnpm run tours`, so a video never shows an
 * older screen than the release it ships with. The storyboard for all ten
 * is docs/developer.book/contributing/videos.md.
 */

import { writeFile } from "node:fs/promises";

import { test as base, type Page } from "@playwright/test";

/** Runs one scene of a tour and captions it. */
export interface Tour {
  scene(caption: string, action: () => Promise<void>): Promise<void>;
}

interface Cue {
  readonly from: number;
  readonly to: number;
  readonly caption: string;
}

/** Long enough to read a caption of about twelve words. */
const READ_MS = 2500;

const pad = (value: number, width = 2) =>
  String(Math.floor(value)).padStart(width, "0");

/** `12345` ms → `00:00:12.345`. */
function timestamp(ms: number): string {
  const hours = pad(ms / 3_600_000);
  const minutes = pad((ms / 60_000) % 60);
  const seconds = pad((ms / 1000) % 60);
  return `${hours}:${minutes}:${seconds}.${pad(ms % 1000, 3)}`;
}

/** The captions file for a tour's cues. */
function webVtt(cues: readonly Cue[]): string {
  const body = cues.map(
    (cue, index) =>
      `${String(index + 1)}\n${timestamp(cue.from)} --> ` +
      `${timestamp(cue.to)}\n${cue.caption}\n`,
  );
  return ["WEBVTT", "", ...body].join("\n");
}

/** Waits so a viewer can read the caption; a tour is watched, not raced. */
async function linger(page: Page) {
  await page.waitForTimeout(READ_MS);
}

/** A Playwright test with a `tour` that captions each scene. */
export const test = base.extend<{ tour: Tour }>({
  tour: async ({ page }, provide, testInfo) => {
    const started = Date.now();
    const cues: Cue[] = [];
    await provide({
      async scene(caption, action) {
        const from = Date.now() - started;
        await action();
        await linger(page);
        cues.push({ from, to: Date.now() - started, caption });
      },
    });
    await writeFile(testInfo.outputPath("captions.vtt"), webVtt(cues));
  },
});

export { expect } from "@playwright/test";
