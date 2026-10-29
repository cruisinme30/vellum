// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Awareness } from "y-protocols/awareness";
import type * as Y from "yjs";

/** The person editing, as peers see their caret. */
export interface CollabUser {
  readonly name: string;
  readonly color: string;
}

/** One open collaborative document. */
export interface CollabSession {
  /** The Yjs document; the editor binds its `default` XML fragment. */
  readonly doc: Y.Doc;
  /** Presence: carets, selections and names of peers. */
  readonly awareness: Awareness;
  /** Resolves once stored and peer state has been applied. */
  readonly ready: Promise<void>;
  /** Leaves the room and frees resources. */
  destroy(): void;
}

/**
 * The collab port. The local connector below syncs browser tabs; the
 * WebSocket connector to vellum-collab replaces it without editor changes.
 */
export interface CollabConnector {
  open(documentId: string): CollabSession;
}

/**
 * Durable local storage for Yjs updates, implemented by the platform
 * layer's DocumentCache (web/src/platform). Updates are opaque bytes.
 */
export interface UpdateCache {
  load(documentId: string): Promise<Uint8Array[]>;
  append(documentId: string, update: Uint8Array): Promise<void>;
}
