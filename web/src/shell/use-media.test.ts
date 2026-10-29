// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, renderHook } from "@testing-library/react";
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { COMPACT, mediaQuery, useMedia } from "./use-media";

/** A matchMedia whose answers the test controls. */
function controllableMedia() {
  const matching = new Set<string>();
  const listeners = new Map<string, Set<() => void>>();
  const matchMedia = vi.fn((query: string) => ({
    get matches() {
      return matching.has(query);
    },
    media: query,
    addEventListener: (_type: string, listener: () => void) => {
      listeners.set(query, (listeners.get(query) ?? new Set()).add(listener));
    },
    removeEventListener: (_type: string, listener: () => void) => {
      listeners.get(query)?.delete(listener);
    },
  }));
  vi.stubGlobal("matchMedia", matchMedia);
  return {
    listeners,
    set(query: string, matches: boolean) {
      if (matches) {
        matching.add(query);
      } else {
        matching.delete(query);
      }
      for (const listener of listeners.get(query) ?? []) {
        listener();
      }
    },
  };
}

/** Removes matchMedia, as on an engine without it; returns the step that puts it back. */
function removeMatchMedia() {
  const original = Object.getOwnPropertyDescriptor(globalThis, "matchMedia");
  Reflect.deleteProperty(globalThis, "matchMedia");
  return () => {
    if (original) {
      Object.defineProperty(globalThis, "matchMedia", original);
    }
  };
}

function CompactProbe() {
  return String(useMedia(COMPACT));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("mediaQuery", () => {
  it("returns the platform's query list", () => {
    expect(mediaQuery(COMPACT)?.media).toBe(COMPACT);
  });

  it("returns undefined where there is no matchMedia", () => {
    const restore = removeMatchMedia();
    try {
      expect(mediaQuery(COMPACT)).toBeUndefined();
    } finally {
      restore();
    }
  });
});

describe("useMedia", () => {
  it("follows the query as it changes and unsubscribes on unmount", () => {
    const media = controllableMedia();
    const { result, unmount } = renderHook(() => useMedia(COMPACT));
    expect(result.current).toBe(false);
    act(() => {
      media.set(COMPACT, true);
    });
    expect(result.current).toBe(true);
    unmount();
    expect(media.listeners.get(COMPACT)?.size).toBe(0);
  });

  it("is false without matchMedia", () => {
    const restore = removeMatchMedia();
    try {
      const { result, unmount } = renderHook(() => useMedia(COMPACT));
      expect(result.current).toBe(false);
      unmount();
    } finally {
      restore();
    }
  });

  it("renders false on the server, whatever the query", () => {
    const media = controllableMedia();
    media.set(COMPACT, true);
    expect(renderToString(createElement(CompactProbe))).toBe("false");
  });
});
