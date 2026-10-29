// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useSyncExternalStore } from "react";

/** Material 3 window size classes. */
export const COMPACT = "(max-width: 599.98px)";
export const MEDIUM = "(max-width: 1023.98px)";

/** A media query list, or `undefined` where the platform has none (tests, old engines). */
export function mediaQuery(query: string): MediaQueryList | undefined {
  return "matchMedia" in globalThis ? globalThis.matchMedia(query) : undefined;
}

/** Whether a media query matches, kept current. */
export function useMedia(query: string): boolean {
  const subscribe = useCallback(
    (callback: () => void) => {
      const list = mediaQuery(query);
      list?.addEventListener("change", callback);
      return () => {
        list?.removeEventListener("change", callback);
      };
    },
    [query],
  );
  return useSyncExternalStore(
    subscribe,
    () => mediaQuery(query)?.matches ?? false,
    () => false,
  );
}
