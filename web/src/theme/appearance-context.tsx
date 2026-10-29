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

import {
  applyAppearance,
  loadAppearance,
  loadReadingFont,
  saveAppearance,
  type Appearance,
} from "./appearance";

/** The organisation's look, from `GET /api/v1/appearance` (R-TH-1, R-TH-4). */
export interface OrganisationAppearance {
  /** The organisation's seed colour, `#rrggbb`. */
  readonly seed: string;
  /** The theme exports use. */
  readonly exportTheme: string;
  /** The theme published sites use; `auto` follows the reader's system. */
  readonly siteTheme: string;
}

interface AppearanceState {
  readonly appearance: Appearance;
  readonly update: (patch: Partial<Appearance>) => void;
  /** The organisation's look; absent in demo mode or until it loads. */
  readonly organisation: OrganisationAppearance | undefined;
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

/**
 * Holds the appearance settings and keeps `<html>` themed. `organisation`
 * loads the organisation's seed colour, which users follow until they pick
 * their own; without it (demo mode) Vellum's seed is used.
 */
export function AppearanceProvider({
  organisation: loadOrganisation,
  children,
}: {
  readonly organisation?: (() => Promise<OrganisationAppearance>) | undefined;
  readonly children: ReactNode;
}) {
  const [appearance, setAppearance] = useState(() =>
    loadAppearance(storage(), document.documentElement.dataset["theme"]),
  );
  const [organisation, setOrganisation] = useState<OrganisationAppearance>();
  const dark = useSyncExternalStore(subscribeDark, prefersDark, () => false);

  useEffect(() => {
    if (loadOrganisation === undefined) return;
    let live = true;
    loadOrganisation().then(
      (value) => {
        if (live) setOrganisation(value);
      },
      () => {
        // The app still works with Vellum's own seed.
      },
    );
    return () => {
      live = false;
    };
  }, [loadOrganisation]);

  useEffect(() => {
    applyAppearance(document.documentElement, appearance, dark, organisation?.seed);
  }, [appearance, dark, organisation]);

  useEffect(() => {
    loadReadingFont(appearance.font);
  }, [appearance.font]);

  const update = useCallback((patch: Partial<Appearance>) => {
    setAppearance((current) => {
      const next = { ...current, ...patch };
      saveAppearance(storage(), next);
      return next;
    });
  }, []);

  const value = useMemo(
    () => ({ appearance, update, organisation }),
    [appearance, update, organisation],
  );
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
