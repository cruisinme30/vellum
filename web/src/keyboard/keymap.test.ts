// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { isEditable, Keymap, type Command, type Scope } from "./keymap";
import type { KeyLike } from "./keys";

function command(id: string, keys: string[], scope: Scope = "global"): Command {
  return { id, title: id, keys, scope, run: vi.fn() };
}

function press(
  key: string,
  extra: Partial<KeyLike> & { target?: unknown } = {},
) {
  const code = /^[a-z]$/.test(key) ? `Key${key.toUpperCase()}` : undefined;
  return {
    key,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...(code ? { code } : {}),
    ...extra,
  };
}

const GLOBAL = new Set<Scope>(["global"]);
const ISSUES = new Set<Scope>(["global", "issues"]);

describe("Keymap", () => {
  it("runs single keys and chords", () => {
    const create = command("create", ["c"]);
    const inbox = command("inbox", ["g i"]);
    const keymap = new Keymap([create, inbox], { platform: "mac" });
    expect(keymap.dispatch(press("c"), GLOBAL)).toEqual({
      kind: "run",
      command: create,
    });
    expect(keymap.dispatch(press("g"), GLOBAL)).toEqual({ kind: "pending" });
    expect(keymap.pending).toBe(true);
    expect(keymap.dispatch(press("i"), GLOBAL)).toEqual({
      kind: "run",
      command: inbox,
    });
    expect(keymap.pending).toBe(false);
    expect(keymap.dispatch(press("z"), GLOBAL)).toEqual({ kind: "none" });
  });

  it("drops a chord after the timeout or a wrong second key", () => {
    let now = 0;
    const inbox = command("inbox", ["g i"]);
    const create = command("create", ["c"]);
    const keymap = new Keymap([inbox, create], {
      platform: "mac",
      chordTimeoutMs: 1000,
      now: () => now,
    });
    keymap.dispatch(press("g"), GLOBAL);
    now = 1500;
    // The chord expired, so `i` alone does nothing.
    expect(keymap.dispatch(press("i"), GLOBAL)).toEqual({ kind: "none" });
    keymap.dispatch(press("g"), GLOBAL);
    // A wrong second key ends the chord without running the single-key command.
    expect(keymap.dispatch(press("c"), GLOBAL)).toEqual({ kind: "none" });
    expect(keymap.dispatch(press("c"), GLOBAL)).toEqual({
      kind: "run",
      command: create,
    });
  });

  it("prefers the more specific scope and ignores inactive ones", () => {
    const cycle = command("cycle", ["shift+c"], "issues");
    const status = command("status", ["s"], "issues");
    const global = command("search", ["s"]);
    const keymap = new Keymap([cycle, status, global], { platform: "other" });
    expect(keymap.dispatch(press("s"), ISSUES)).toEqual({
      kind: "run",
      command: status,
    });
    expect(keymap.dispatch(press("s"), GLOBAL)).toEqual({
      kind: "run",
      command: global,
    });
    expect(
      keymap.dispatch(press("C", { code: "KeyC", shiftKey: true }), GLOBAL),
    ).toEqual({
      kind: "none",
    });
    expect(
      keymap.dispatch(press("C", { code: "KeyC", shiftKey: true }), ISSUES),
    ).toEqual({
      kind: "run",
      command: cycle,
    });
    expect(keymap.bindingsOf(cycle)).toHaveLength(1);
  });

  it(
    "lets typing through but still honours mod shortcuts in text " + "fields",
    () => {
      const create = command("create", ["c"]);
      const menu = command("menu", ["mod+k"]);
      const keymap = new Keymap([create, menu], { platform: "mac" });
      const input = document.createElement("input");
      expect(keymap.dispatch(press("c", { target: input }), GLOBAL)).toEqual({
        kind: "none",
      });
      expect(
        keymap.dispatch(press("k", { metaKey: true, target: input }), GLOBAL),
      ).toEqual({
        kind: "run",
        command: menu,
      });
    },
  );

  it("ignores events it cannot read", () => {
    const keymap = new Keymap([command("menu", ["mod+k"])], {
      platform: "mac",
    });
    expect(keymap.dispatch(press("Meta", { metaKey: true }), GLOBAL)).toEqual({
      kind: "none",
    });
  });

  it("refuses clashing bindings", () => {
    expect(
      () =>
        new Keymap([command("a", ["c"]), command("b", ["c"])], {
          platform: "mac",
        }),
    ).toThrow(/share/);
    expect(
      () =>
        new Keymap([command("go", ["g i"]), command("g", ["g"], "issues")], {
          platform: "mac",
        }),
    ).toThrow(/first key/);
    // The same key in different scopes is fine; the specific scope wins.
    expect(
      () =>
        new Keymap([command("a", ["c"]), command("b", ["c"], "issues")], {
          platform: "mac",
        }),
    ).not.toThrow();
    // A chord and a single key in two specific scopes never meet.
    expect(
      () =>
        new Keymap(
          [command("a", ["g i"], "editor"), command("b", ["g"], "issues")],
          {
            platform: "mac",
          },
        ),
    ).not.toThrow();
  });

  it("dispatches well inside the 100 ms latency budget", () => {
    const commands = Array.from({ length: 200 }, (_, index) =>
      command(`c${String(index)}`, [], "global"),
    );
    commands.push(command("create", ["c"]), command("inbox", ["g i"]));
    const keymap = new Keymap(commands, { platform: "mac" });
    const started = performance.now();
    for (let index = 0; index < 1000; index += 1) {
      keymap.dispatch(press("c"), GLOBAL);
    }
    // A thousand presses take far less than one frame each.
    expect((performance.now() - started) / 1000).toBeLessThan(1);
  });
});

describe("isEditable", () => {
  it("knows fields and editable content", () => {
    expect(isEditable(document.createElement("textarea"))).toBe(true);
    expect(isEditable(document.createElement("select"))).toBe(true);
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    const child = document.createElement("p");
    editor.append(child);
    expect(isEditable(child)).toBe(true);
    expect(isEditable(document.createElement("button"))).toBe(false);
    expect(isEditable("text")).toBe(false);
  });
});
