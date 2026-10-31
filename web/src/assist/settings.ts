// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The person's own model settings, kept in this browser only (ADR 0056,
 * D238): in session storage by default, in local storage when they choose
 * "Remember on this device". The server never stores the key.
 */

import { useCallback, useSyncExternalStore } from "react";

import type { ModelSettings, Provider } from "../api/api.gen";

/** The settings as the form edits them. */
export interface StoredModel {
  readonly provider: Provider;
  /** Empty for the provider's public API. */
  readonly baseUrl: string;
  readonly model: string;
  /** Empty for a self-hosted endpoint that needs none. */
  readonly apiKey: string;
  readonly remember: boolean;
}

/** The model suggested for each provider. */
export const SUGGESTED_MODEL: Readonly<Record<Provider, string>> = {
  anthropic: "claude-opus-5-5",
  openai_compatible: "",
};

const PREFIX = "vellum.assist.";

type Area = "local" | "session";

function area(which: Area): Storage | undefined {
  try {
    return which === "local"
      ? globalThis.localStorage
      : globalThis.sessionStorage;
  } catch {
    return undefined;
  }
}

function read(which: Area, key: string): StoredModel | undefined {
  try {
    const text = area(which)?.getItem(key);
    if (text === null || text === undefined) return undefined;
    const value = JSON.parse(text) as Partial<StoredModel>;
    if (
      (value.provider !== "anthropic" &&
        value.provider !== "openai_compatible") ||
      typeof value.model !== "string"
    ) {
      return undefined;
    }
    return {
      provider: value.provider,
      baseUrl: typeof value.baseUrl === "string" ? value.baseUrl : "",
      model: value.model,
      apiKey: typeof value.apiKey === "string" ? value.apiKey : "",
      remember: which === "local",
    };
  } catch {
    return undefined;
  }
}

const listeners = new Set<() => void>();
const cache = new Map<string, StoredModel | undefined>();

function notify() {
  cache.clear();
  for (const listener of listeners) listener();
}

/** The saved settings for `userId`, if any. */
export function loadModel(userId: string): StoredModel | undefined {
  const key = PREFIX + userId;
  if (!cache.has(key)) {
    cache.set(key, read("session", key) ?? read("local", key));
  }
  return cache.get(key);
}

/** Saves the settings for `userId` where `remember` says. */
export function saveModel(userId: string, value: StoredModel) {
  const key = PREFIX + userId;
  const { remember, ...kept } = value;
  try {
    area(remember ? "local" : "session")?.setItem(key, JSON.stringify(kept));
    area(remember ? "session" : "local")?.removeItem(key);
  } catch {
    // Storage is off (private window): the settings last until reload.
    cache.set(key, value);
    for (const listener of listeners) listener();
    return;
  }
  notify();
}

/** Removes the settings, and so the key, for `userId`. */
export function forgetModel(userId: string) {
  const key = PREFIX + userId;
  try {
    area("local")?.removeItem(key);
    area("session")?.removeItem(key);
  } catch {
    // Nothing stored.
  }
  notify();
  cache.set(key, undefined);
}

/** What the API takes. */
export function toSettings(stored: StoredModel): ModelSettings {
  return {
    provider: stored.provider,
    model: stored.model.trim(),
    ...(stored.baseUrl.trim() === ""
      ? {}
      : { base_url: stored.baseUrl.trim() }),
    ...(stored.apiKey.trim() === "" ? {} : { api_key: stored.apiKey.trim() }),
  };
}

/** The key's last four characters, for showing which key is saved. */
export function keyHint(key: string): string {
  return key.length <= 8 ? "••••" : `••••${key.slice(-4)}`;
}

/** The saved settings for `userId`, updating when they change. */
export function useStoredModel(
  userId: string | undefined,
): StoredModel | undefined {
  const subscribe = useCallback((listener: () => void) => {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, []);
  return useSyncExternalStore(subscribe, () =>
    userId === undefined ? undefined : loadModel(userId),
  );
}
