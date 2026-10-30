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
  /**
   * The server connection, for sessions backed by one. Local sessions
   * leave it out: they are always editable and have nothing to save.
   */
  readonly live?: LiveConnection;
}

/** Where a server-backed session stands. */
type CollabStatus =
  /** Opening the socket, or waiting to retry. */
  | "connecting"
  /** Synced with the room; edits reach the server as they are made. */
  | "online"
  /** The socket dropped; edits are kept and sent when it comes back. */
  | "offline"
  /** The server ended access to the page; the session will not reconnect. */
  | "revoked";

/** A snapshot of a server-backed session. */
export interface LiveState {
  readonly status: CollabStatus;
  /**
   * Whether this editor may change the page, as the server says;
   * `undefined` until the server has said, so a page that is still joining
   * is not taken for a read-only one.
   */
  readonly canWrite: boolean | undefined;
  /** The signed-in user behind each awareness client ID, as the server vouches. */
  readonly peers: ReadonlyMap<number, { readonly user: string; readonly canWrite: boolean }>;
}

/** The server side of a session, readable with `useSyncExternalStore`. */
export interface LiveConnection {
  readonly getSnapshot: () => LiveState;
  readonly subscribe: (listener: () => void) => () => void;
  /**
   * Asks the server to write a version now (the editor's Done). Resolves
   * when the server confirms; rejects when the socket is not online or
   * closes first.
   */
  readonly done: () => Promise<void>;
}

/**
 * The collab port. The local connector syncs browser tabs in demo mode;
 * the socket connector joins the server's room at /collab when a Vellum
 * server answers. The editor works the same with either.
 */
export interface CollabConnector {
  /** Opens a page's document. `spaceId` names the drive space that holds it. */
  open(documentId: string, spaceId: string): CollabSession;
}

/**
 * Durable local storage for Yjs updates, implemented by the platform
 * layer's DocumentCache (web/src/platform). Updates are opaque bytes.
 */
export interface UpdateCache {
  load(documentId: string): Promise<Uint8Array[]>;
  append(documentId: string, update: Uint8Array): Promise<void>;
}
