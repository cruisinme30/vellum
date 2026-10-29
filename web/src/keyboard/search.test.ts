// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { Command } from "./keymap";
import { rankCommands, score } from "./search";

function command(title: string, keywords: string[] = []): Command {
  return { id: title, title, keys: [], scope: "global", keywords, run: vi.fn() };
}

describe("score", () => {
  it("ranks a prefix over a word start over letters in order", () => {
    const prefix = score("Set priority", "set");
    const word = score("Set priority", "prio");
    const inside = score("Set priority", "riority");
    const scattered = score("Set priority", "stpr");
    expect(prefix).toBeGreaterThan(word);
    expect(word).toBeGreaterThan(inside);
    expect(inside).toBeGreaterThan(scattered);
    expect(scattered).toBeGreaterThan(0);
    expect(score("Set priority", "zz")).toBe(0);
    expect(score("Anything", "  ")).toBe(1);
  });
});

describe("rankCommands", () => {
  const commands = [
    command("New issue", ["ticket"]),
    command("Go to Inbox"),
    command("Assign to me"),
  ];

  it("keeps the order for an empty query", () => {
    expect(rankCommands(commands, "").map((c) => c.title)).toEqual([
      "New issue",
      "Go to Inbox",
      "Assign to me",
    ]);
  });

  it("matches titles and keywords, best first", () => {
    expect(rankCommands(commands, "in").map((c) => c.title)).toEqual([
      "Go to Inbox",
      "Assign to me",
    ]);
    expect(rankCommands(commands, "is").map((c) => c.title)).toEqual(["New issue"]);
    expect(rankCommands(commands, "ticket").map((c) => c.title)).toEqual(["New issue"]);
    expect(rankCommands(commands, "xyz")).toEqual([]);
  });
});
