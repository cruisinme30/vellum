// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useSyncExternalStore } from "react";

function subscribe(listener: () => void): () => void {
  globalThis.addEventListener("online", listener);
  globalThis.addEventListener("offline", listener);
  return () => {
    globalThis.removeEventListener("online", listener);
    globalThis.removeEventListener("offline", listener);
  };
}

const online = () => navigator.onLine;
const serverSnapshot = () => true;

/**
 * Whether the browser believes it is online, for the offline banner. The
 * local-first store keeps working either way; this only informs people.
 */
export function useOnline(): boolean {
  return useSyncExternalStore(subscribe, online, serverSnapshot);
}

/** Subscribes the sync scheduler (or anything else) to connectivity changes. */
export function watchConnectivity(onChange: (online: boolean) => void): () => void {
  return subscribe(() => {
    onChange(navigator.onLine);
  });
}
