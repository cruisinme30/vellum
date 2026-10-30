// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type * as Y from "yjs";

import type { LiveConnection } from "./provider";

/** Where this editor's own changes stand. */
export type SaveStatus =
  /** Every change this editor made is in a written version. */
  | "saved"
  /** Changes made since the last version; one is written on idle or Done. */
  | "unsaved"
  /** Waiting for the server to write a version. */
  | "saving"
  /** The last save failed; the changes are kept and the next one retries. */
  | "failed";

/** Options of {@link createSaver}. */
export interface SaverOptions {
  /** How long after this editor's last change a version is written. */
  readonly idleMs?: number;
}

/** The save state of one open page, readable with `useSyncExternalStore`. */
export interface Saver {
  readonly getSnapshot: () => SaveStatus;
  readonly subscribe: (listener: () => void) => () => void;
  /** Writes a version now (the Done button). Never rejects. */
  readonly save: () => Promise<void>;
  /** Stops the idle timer and listening. */
  readonly dispose: () => void;
}

/**
 * The brief's default: a version is written after 30 seconds without
 * edits. The server's room keeps the same timer for every editor; this
 * one only lets the editor show when its own changes are safely written.
 */
const IDLE_SAVE_MS = 30_000;

/**
 * Tracks this editor's own changes and writes a version through the
 * session's Done when the editor goes quiet, or when asked. A Done with
 * nothing new is only acknowledged by the server, so the room's idle
 * version and this one never make two copies of the same text.
 *
 * Only local transactions count: updates from the server and the seed
 * arrive through `Y.applyUpdate`, which Yjs marks as remote.
 */
export function createSaver(
  document_: Y.Doc,
  live: LiveConnection,
  options: SaverOptions = {},
): Saver {
  const idleMs = options.idleMs ?? IDLE_SAVE_MS;
  let status: SaveStatus = "saved";
  /** Bumped by every local change, so a save knows whether it covered them all. */
  let edits = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  /** An object, so checks after an `await` are not narrowed away. */
  const life = { disposed: false };
  const listeners = new Set<() => void>();

  const set = (next: SaveStatus) => {
    if (next === status) {
      return;
    }
    status = next;
    for (const listener of listeners) {
      listener();
    }
  };

  const schedule = () => {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = undefined;
      void save();
    }, idleMs);
  };

  const canSave = () => {
    const state = live.getSnapshot();
    return state.status === "online" && state.canWrite === true;
  };

  /** Records how a save that covered `covered` edits ended. */
  const settle = (covered: number, ok: boolean) => {
    if (life.disposed) {
      return;
    }
    if (ok && edits === covered) {
      set("saved");
      return;
    }
    set(ok ? "unsaved" : "failed");
    schedule();
  };

  /**
   * Writes a version; `force` asks even when this editor has nothing new.
   * Offline edits reach the server on reconnect, and the reconnect handler
   * below schedules the version then.
   */
  const save = async (force = false) => {
    if (life.disposed || status === "saving" || (status === "saved" && !force) || !canSave()) {
      return;
    }
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    const covered = edits;
    set("saving");
    const ok = await live.done().then(
      () => true,
      () => false,
    );
    settle(covered, ok);
  };

  const onTransaction = (transaction: Y.Transaction) => {
    if (!transaction.local || transaction.changed.size === 0) {
      return;
    }
    edits += 1;
    if (status !== "saving") {
      set("unsaved");
    }
    schedule();
  };
  document_.on("afterTransaction", onTransaction);

  let wasOnline = live.getSnapshot().status === "online";
  const unsubscribe = live.subscribe(() => {
    const online = live.getSnapshot().status === "online";
    if (online && !wasOnline && status !== "saved") {
      schedule();
    }
    wasOnline = online;
  });

  return {
    getSnapshot: () => status,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    // Done with nothing new still asks, so a peer's unsaved typing is
    // written too; the server only acknowledges when nothing changed.
    save: () => save(true),
    dispose() {
      life.disposed = true;
      if (timer !== undefined) {
        clearTimeout(timer);
      }
      document_.off("afterTransaction", onTransaction);
      unsubscribe();
      listeners.clear();
    },
  };
}
