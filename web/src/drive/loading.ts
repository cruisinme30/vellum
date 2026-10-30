// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useState } from "react";

import { failureText } from "../data/failure";
import { useLoaded, type Loaded } from "../data/use-loaded";

import type { DriveApi, NodeView, SpaceView } from "./api";

const sameKeys = (a: readonly unknown[], b: readonly unknown[]) =>
  a.length === b.length && a.every((key, index) => Object.is(key, b[index]));

/**
 * Loads like `useLoaded`, and can load again on `reload()`. While a reload
 * is on its way the last answer for the same keys stays on screen, so a
 * list does not blink, and keeps its notices, after every change.
 */
export function useReloadable<T>(
  load: () => Promise<T>,
  keys: readonly unknown[],
): { readonly loaded: Loaded<T>; readonly reload: () => void } {
  const [version, setVersion] = useState(0);
  const loaded = useLoaded(load, [...keys, version], failureText);
  const [kept, setKept] = useState<{ keys: readonly unknown[]; value: Loaded<T> }>();
  if (loaded.state !== "loading" && kept?.value !== loaded) {
    setKept({ keys, value: loaded });
  }
  const shown =
    loaded.state === "loading" && kept !== undefined && sameKeys(kept.keys, keys)
      ? kept.value
      : loaded;
  const reload = useCallback(() => {
    setVersion((value) => value + 1);
  }, []);
  return { loaded: shown, reload };
}

/** Items with the IDs of the ones the person starred. */
export interface Listing {
  readonly items: NodeView[];
  readonly starred: ReadonlySet<string>;
}

/** Loads some items together with the person's stars. */
export async function withStars(api: DriveApi, items: Promise<NodeView[]>): Promise<Listing> {
  const [found, starred] = await Promise.all([items, api.starred()]);
  return { items: found, starred: new Set(starred.map((node) => node.id)) };
}

/** The spaces the person can open: their My Drive first, then the shared drives. */
export async function loadSpaces(api: DriveApi): Promise<SpaceView[]> {
  const [mine, shared] = await Promise.all([api.myDrive(), api.sharedDrives()]);
  return mine === undefined ? shared : [mine, ...shared.filter((space) => space.id !== mine.id)];
}

/** The folders above a folder, top first, ending with the folder itself. */
export async function ancestors(api: DriveApi, folderId: string): Promise<NodeView[]> {
  const chain: NodeView[] = [];
  let next: string | undefined = folderId;
  for (let step = 0; next !== undefined && step < 32; step += 1) {
    const node = await api.get(next);
    chain.unshift(node);
    next = node.parent_id ?? undefined;
  }
  return chain;
}
