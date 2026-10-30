// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { Keymap } from "./keymap";
import {
  APP_DESTINATIONS,
  SCREEN_DESTINATIONS,
  workCommands,
  type WorkActions,
} from "./work-commands";

function everything(): Required<Omit<WorkActions, "destinations">> {
  return {
    newIssue: vi.fn(),
    newIssueFromTemplate: vi.fn(),
    quickAddTask: vi.fn(),
    goTo: vi.fn(),
    setStatus: vi.fn(),
    setPriority: vi.fn(),
    setLabels: vi.fn(),
    assign: vi.fn(),
    assignToMe: vi.fn(),
    setEstimate: vi.fn(),
    setCycle: vi.fn(),
    toggleSelect: vi.fn(),
    peek: vi.fn(),
    copyBranchName: vi.fn(),
  };
}

describe("workCommands", () => {
  it("binds every key on the shortcut page without clashes", () => {
    const commands = workCommands(everything());
    expect(commands).toHaveLength(18);
    // The keymap refuses clashing bindings, so building one is the check.
    expect(() => new Keymap(commands, { platform: "mac" })).not.toThrow();
    const keys = Object.fromEntries(commands.map((c) => [c.id, c.keys[0]]));
    expect(keys).toMatchObject({
      "issue.create": "c",
      "issue.create-from-template": "alt+c",
      "issue.set-cycle": "shift+c",
      "issue.peek": "space",
      "issue.copy-branch": "mod+shift+.",
      "go.inbox": "g i",
      "go.triage": "g t",
    });
  });

  it("leaves out what the screen cannot do", () => {
    const newIssue = vi.fn();
    const commands = workCommands({ newIssue });
    expect(commands.map((c) => c.id)).toEqual(["issue.create"]);
    commands[0]?.run();
    expect(newIssue).toHaveBeenCalledOnce();
  });

  it("sends the G chords to their destination", () => {
    const actions = everything();
    const inbox = workCommands(actions).find((c) => c.id === "go.my-work");
    inbox?.run();
    expect(actions.goTo).toHaveBeenCalledWith("my-work");
  });

  it("leaves out the G chords for screens the app does not have", () => {
    const goTo = vi.fn();
    const commands = workCommands({ goTo, destinations: ["inbox", "my-work"] });
    expect(commands.map((command) => command.id)).toEqual([
      "go.inbox",
      "go.my-work",
    ]);
  });

  it("lets the app and a Work screen bind their G chords side by side", () => {
    // Regression: both registered `G I` globally, so every Work screen threw.
    const app = workCommands({ goTo: vi.fn(), destinations: APP_DESTINATIONS });
    const screen = workCommands({
      ...everything(),
      destinations: SCREEN_DESTINATIONS,
    });
    expect(
      () => new Keymap([...app, ...screen], { platform: "mac" }),
    ).not.toThrow();
    const all = [...APP_DESTINATIONS, ...SCREEN_DESTINATIONS];
    expect(new Set(all).size).toBe(all.length);
    expect(new Set(all)).toEqual(
      new Set(["board", "cycle", "inbox", "my-work", "triage"]),
    );
  });
});
