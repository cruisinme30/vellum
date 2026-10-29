// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Browser APIs jsdom lacks, stubbed for every unit test.
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  cleanup();
  localStorage.clear();
});

const noop = () => {
  // A stub listener method.
};

// jsdom defines matchMedia as undefined, so test the type, not the key.
if (typeof (globalThis as { matchMedia?: unknown }).matchMedia !== "function") {
  Object.defineProperty(globalThis, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      addEventListener: noop,
      removeEventListener: noop,
      addListener: noop,
      removeListener: noop,
      dispatchEvent: () => false,
    }),
  });
}

// jsdom has no layout; ProseMirror measures text with ranges.
const stubs: [object, string, unknown][] = [
  [Element.prototype, "scrollIntoView", noop],
  [Range.prototype, "getClientRects", () => []],
  [Range.prototype, "getBoundingClientRect", () => new DOMRect()],
  [document, "elementFromPoint", noop],
];
for (const [target, name, value] of stubs) {
  if (typeof (target as Record<string, unknown>)[name] !== "function") {
    Object.defineProperty(target, name, { configurable: true, value });
  }
}
