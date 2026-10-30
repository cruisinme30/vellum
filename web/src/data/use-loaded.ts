// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useState } from "react";

/** A value being loaded: still on its way, failed with a message, or here. */
export type Loaded<T> =
  | { readonly state: "loading" }
  | { readonly state: "failed"; readonly message: string }
  | { readonly state: "ready"; readonly value: T };

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

/**
 * Loads a promise's answer into state, again whenever `keys` change; until
 * the answer for the current keys arrives it reads as loading. `describe`
 * turns a failure into the message shown.
 */
export function useLoaded<T>(
  load: () => Promise<T>,
  keys: readonly unknown[],
  describe: (error: unknown) => string = errorText,
): Loaded<T> {
  const [loaded, setLoaded] = useState<{
    keys: readonly unknown[];
    value: Loaded<T>;
  }>();
  useEffect(() => {
    let current = true;
    load().then(
      (value) => {
        if (current) setLoaded({ keys, value: { state: "ready", value } });
      },
      (error: unknown) => {
        if (current)
          setLoaded({
            keys,
            value: { state: "failed", message: describe(error) },
          });
      },
    );
    return () => {
      current = false;
    };
    // The keys say when to load again; `load` is a fresh closure each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, keys);
  const same =
    loaded?.keys.length === keys.length &&
    loaded.keys.every((key, index) => Object.is(key, keys[index]));
  return same ? loaded.value : { state: "loading" };
}
