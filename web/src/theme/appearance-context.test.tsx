// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppearanceProvider, useAppearance } from "./appearance-context";

/** Swallows React's logged error for an expected throw. */
const quiet = () => {
  // Expected error output.
};

const wrapper = ({ children }: { readonly children: ReactNode }) => (
  <AppearanceProvider>{children}</AppearanceProvider>
);

/** A controllable `(prefers-color-scheme: dark)` media query. */
function fakeDarkQuery(initial: boolean) {
  const listeners = new Set<() => void>();
  const list = {
    matches: initial,
    addEventListener: (_type: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_type: string, listener: () => void) => listeners.delete(listener),
  };
  vi.stubGlobal("matchMedia", () => list);
  return {
    listeners,
    set(matches: boolean) {
      list.matches = matches;
      for (const listener of listeners) {
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

function ThemeName() {
  return useAppearance().appearance.theme;
}

afterEach(() => {
  vi.unstubAllGlobals();
  delete document.documentElement.dataset["theme"];
});

describe("AppearanceProvider", () => {
  it("starts from saved settings and themes <html>", () => {
    localStorage.setItem(
      "vellum.appearance",
      JSON.stringify({ theme: "dim", seed: "#4f5b92", density: "compact" }),
    );
    const { result } = renderHook(() => useAppearance(), { wrapper });
    expect(result.current.appearance.theme).toBe("dim");
    expect(document.documentElement.dataset["theme"]).toBe("dim");
    expect(document.documentElement.dataset["density"]).toBe("compact");
  });

  it("updates, saves and re-applies settings", () => {
    const { result } = renderHook(() => useAppearance(), { wrapper });
    act(() => {
      result.current.update({ theme: "sepia" });
    });
    expect(result.current.appearance.theme).toBe("sepia");
    expect(document.documentElement.dataset["theme"]).toBe("sepia");
    expect(JSON.parse(localStorage.getItem("vellum.appearance") ?? "{}")).toMatchObject({
      theme: "sepia",
    });
  });

  it("follows the OS dark preference for the system theme, and unsubscribes", () => {
    const query = fakeDarkQuery(false);
    const { unmount } = renderHook(() => useAppearance(), { wrapper });
    expect(document.documentElement.dataset["theme"]).toBe("light");
    act(() => {
      query.set(true);
    });
    expect(document.documentElement.dataset["theme"]).toBe("dark");
    unmount();
    expect(query.listeners.size).toBe(0);
  });

  it("works without matchMedia or localStorage", () => {
    const restoreMatchMedia = removeMatchMedia();
    const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      get() {
        throw new Error("SecurityError");
      },
    });
    try {
      const { result } = renderHook(() => useAppearance(), { wrapper });
      act(() => {
        result.current.update({ density: "compact" });
      });
      expect(result.current.appearance.density).toBe("compact");
      expect(document.documentElement.dataset["theme"]).toBe("light");
    } finally {
      if (original) {
        Object.defineProperty(globalThis, "localStorage", original);
      }
      restoreMatchMedia();
    }
  });
});

describe("server rendering", () => {
  it("renders on the server without reading the OS preference", () => {
    fakeDarkQuery(true);
    expect(
      renderToString(
        <AppearanceProvider>
          <ThemeName />
        </AppearanceProvider>,
      ),
    ).toBe("system");
  });
});

describe("useAppearance", () => {
  it("throws outside a provider", () => {
    const silence = vi.spyOn(console, "error").mockImplementation(quiet);
    expect(() => renderHook(() => useAppearance())).toThrow(/needs an AppearanceProvider/);
    silence.mockRestore();
  });
});
