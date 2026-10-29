// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  currentPlatform,
  formatBinding,
  parseBinding,
  sameStroke,
  strokeOf,
  type KeyLike,
} from "./keys";

function press(key: string, extra: Partial<KeyLike> = {}): KeyLike {
  return { key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...extra };
}

describe("parseBinding", () => {
  it("reads modifiers, named keys and chords", () => {
    expect(parseBinding("mod+k")).toEqual([{ key: "k", mod: true, alt: false, shift: false }]);
    expect(parseBinding("g i")).toHaveLength(2);
    expect(parseBinding("shift+C")[0]).toEqual({ key: "c", mod: false, alt: false, shift: true });
    expect(parseBinding("mod+plus")[0]?.key).toBe("+");
    expect(parseBinding("mod+shift+.")[0]).toEqual({
      key: ".",
      mod: true,
      alt: false,
      shift: true,
    });
  });

  it("refuses bindings it cannot match", () => {
    expect(() => parseBinding("hyper+k")).toThrow(/Invalid/);
    expect(() => parseBinding("mod+")).toThrow(/Invalid/);
    expect(() => parseBinding("g g g")).toThrow(/at most two/);
    expect(() => parseBinding("shift+?")).toThrow(/without shift/);
  });
});

describe("strokeOf", () => {
  it("uses ⌘ as mod on a Mac and Ctrl elsewhere", () => {
    expect(strokeOf(press("k", { metaKey: true }), "mac")?.mod).toBe(true);
    expect(strokeOf(press("k", { ctrlKey: true }), "other")?.mod).toBe(true);
    // The other platform's key is left to the system.
    expect(strokeOf(press("k", { ctrlKey: true }), "mac")).toBeUndefined();
    expect(strokeOf(press("k", { metaKey: true }), "other")).toBeUndefined();
  });

  it("ignores lone modifiers", () => {
    expect(strokeOf(press("Shift", { shiftKey: true }), "mac")).toBeUndefined();
  });

  it("reads letters from the physical key so Alt C works on a Mac", () => {
    const stroke = strokeOf(press("ç", { altKey: true, code: "KeyC" }), "mac");
    expect(stroke).toEqual({ key: "c", mod: false, alt: true, shift: false });
    expect(strokeOf(press("1", { code: "Digit1" }), "other")?.key).toBe("1");
  });

  it("counts shift for letters but not for punctuation", () => {
    expect(strokeOf(press("C", { shiftKey: true, code: "KeyC" }), "mac")?.shift).toBe(true);
    expect(strokeOf(press("?", { shiftKey: true, code: "Slash" }), "mac")).toEqual({
      key: "?",
      mod: false,
      alt: false,
      shift: false,
    });
  });

  it("reads punctuation from the physical key under mod", () => {
    const stroke = strokeOf(press(">", { metaKey: true, shiftKey: true, code: "Period" }), "mac");
    expect(stroke && sameStroke(stroke, parseBinding("mod+shift+.")[0] ?? stroke)).toBe(true);
  });

  it("names the space bar and other named keys", () => {
    expect(strokeOf(press(" ", { code: "Space" }), "mac")?.key).toBe("space");
    expect(strokeOf(press("Escape"), "mac")?.key).toBe("escape");
  });
});

describe("formatBinding", () => {
  it("shows glyphs on a Mac and words elsewhere", () => {
    expect(formatBinding(parseBinding("mod+k"), "mac")).toBe("⌘K");
    expect(formatBinding(parseBinding("mod+k"), "other")).toBe("Ctrl+K");
    expect(formatBinding(parseBinding("alt+shift+c"), "other")).toBe("Alt+Shift+C");
    expect(formatBinding(parseBinding("mod+alt+shift+m"), "mac")).toBe("⌘⌥⇧M");
    expect(formatBinding(parseBinding("g i"), "mac")).toBe("G then I");
    expect(formatBinding(parseBinding("space"), "mac")).toBe("Space");
  });
});

describe("currentPlatform", () => {
  it("detects Apple devices", () => {
    expect(currentPlatform({ platform: "MacIntel" })).toBe("mac");
    expect(currentPlatform({ userAgent: "Mozilla/5.0 (iPad; CPU OS 18_0)" })).toBe("mac");
    expect(currentPlatform({ platform: "Win32" })).toBe("other");
    expect(currentPlatform({})).toBe("other");
    expect(["mac", "other"]).toContain(currentPlatform());
  });
});
