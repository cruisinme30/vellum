// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import { mediaQuery } from "../shell/use-media";

import { applyAppearance, loadAppearance, saveAppearance, type Appearance } from "./appearance";

interface AppearanceState {
  readonly appearance: Appearance;
  readonly update: (patch: Partial<Appearance>) => void;
}

const AppearanceContext = createContext<AppearanceState | undefined>(undefined);

const DARK_QUERY = "(prefers-color-scheme: dark)";

function subscribeDark(callback: () => void) {
  const query = mediaQuery(DARK_QUERY);
  query?.addEventListener("change", callback);
  return () => {
    query?.removeEventListener("change", callback);
  };
}

function prefersDark() {
  return mediaQuery(DARK_QUERY)?.matches ?? false;
}

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

/** Holds the appearance settings and keeps `<html>` themed. */
export function AppearanceProvider({ children }: { readonly children: ReactNode }) {
  const [appearance, setAppearance] = useState(() =>
    loadAppearance(storage(), document.documentElement.dataset["theme"]),
  );
  const dark = useSyncExternalStore(subscribeDark, prefersDark, () => false);

  useEffect(() => {
    applyAppearance(document.documentElement, appearance, dark);
  }, [appearance, dark]);

  const update = useCallback((patch: Partial<Appearance>) => {
    setAppearance((current) => {
      const next = { ...current, ...patch };
      saveAppearance(storage(), next);
      return next;
    });
  }, []);

  const value = useMemo(() => ({ appearance, update }), [appearance, update]);
  return <AppearanceContext value={value}>{children}</AppearanceContext>;
}

/** The current appearance settings and a setter. */
export function useAppearance(): AppearanceState {
  const value = use(AppearanceContext);
  if (value === undefined) {
    throw new Error("useAppearance needs an AppearanceProvider");
  }
  return value;
}
