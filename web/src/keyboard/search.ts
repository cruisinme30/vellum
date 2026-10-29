// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Ranking for the command menu: typed letters in order, best matches first. */

import type { Command } from "./keymap";

/**
 * How well `query` matches `text`: higher is better, `0` is no match.
 * A prefix beats a word start, which beats letters in order.
 */
export function score(text: string, query: string): number {
  const haystack = text.toLowerCase();
  const needle = query.trim().toLowerCase();
  if (needle === "") {
    return 1;
  }
  if (haystack.startsWith(needle)) {
    return 100 - Math.min(haystack.length - needle.length, 50) / 100;
  }
  const at = haystack.indexOf(needle);
  if (at > 0 && /\W/.test(haystack.charAt(at - 1))) {
    return 80;
  }
  if (at > 0) {
    return 60;
  }
  let from = 0;
  let gaps = 0;
  for (const char of needle) {
    const found = haystack.indexOf(char, from);
    if (found === -1) {
      return 0;
    }
    gaps += found - from;
    from = found + 1;
  }
  return Math.max(1, 40 - gaps);
}

/** The commands matching `query`, best first; ties keep their order. */
export function rankCommands(commands: readonly Command[], query: string): Command[] {
  return commands
    .map((command, index) => ({
      command,
      index,
      score: Math.max(
        score(command.title, query),
        ...(command.keywords ?? []).map((k) => score(k, query) - 5),
      ),
    }))
    .filter((r) => r.score > 0)
    .toSorted((a, b) => b.score - a.score || a.index - b.index)
    .map((r) => r.command);
}
