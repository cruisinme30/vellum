// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  applyAwarenessUpdate,
  Awareness,
  encodeAwarenessUpdate,
  removeAwarenessStates,
} from "y-protocols/awareness";
import * as Y from "yjs";

import type { CollabConnector, CollabSession, LiveConnection, LiveState } from "./provider";
import {
  decodeFrame,
  decodePeers,
  encodeFrame,
  MSG_DONE,
  MSG_PEERS,
  WireError,
  type WireMessage,
} from "./wire";

/** The parts of a browser WebSocket the connector uses; injectable for tests. */
export interface SocketLike {
  binaryType: BinaryType;
  readonly readyState: number;
  addEventListener<K extends "open" | "message" | "close">(
    type: K,
    listener: (event: WebSocketEventMap[K]) => void,
  ): void;
  send(data: Uint8Array<ArrayBuffer>): void;
  close(code?: number, reason?: string): void;
}

/** Opens a socket to a URL. */
type SocketFactory = (url: string) => SocketLike;

/** Options of {@link createSocketConnector}. */
export interface SocketConnectorOptions {
  /** The page origin the socket URL is built from; defaults to `location.origin`. */
  readonly origin?: string;
  readonly socket?: SocketFactory;
  /** A number in [0, 1) that spreads reconnects out; defaults to `Math.random`. */
  readonly random?: () => number;
  /** How long Done waits for the server's answer. */
  readonly doneTimeoutMs?: number;
}

const OPEN = 1;
/** The server's close code when the user's access was revoked (`CloseReason::Revoked`). */
const CLOSE_REVOKED = 4403;
const RETRY_FIRST_MS = 500;
const RETRY_MAX_MS = 30_000;
const DONE_TIMEOUT_MS = 15_000;

const REMOTE = Symbol("remote");

/** The socket URL of a page's room: `/collab/{space_id}/{doc_id}` on the app's origin. */
export function collabUrl(origin: string, spaceId: string, documentId: string): string {
  const url = new URL(
    `/collab/${encodeURIComponent(spaceId)}/${encodeURIComponent(documentId)}`,
    origin,
  );
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

/**
 * Joins pages' live-editing rooms on the Vellum server (ADR 0009) over
 * WebSocket. The session cookie authenticates the handshake, and the
 * server only admits it from the app's own origin.
 *
 * A dropped socket reconnects with backoff. Edits made meanwhile stay in
 * the document; the sync exchange on reconnect sends whatever the server
 * lacks, so nothing is queued by hand.
 */
export function createSocketConnector(options: SocketConnectorOptions = {}): CollabConnector {
  const origin = options.origin ?? globalThis.location.origin;
  const socketFor: SocketFactory = options.socket ?? ((url) => new WebSocket(url));
  const random = options.random ?? Math.random;
  const doneTimeoutMs = options.doneTimeoutMs ?? DONE_TIMEOUT_MS;

  return {
    open(documentId, spaceId): CollabSession {
      const url = collabUrl(origin, spaceId, documentId);
      const document_ = new Y.Doc();
      const awareness = new Awareness(document_);

      let state: LiveState = { status: "connecting", canWrite: false, peers: new Map() };
      const listeners = new Set<() => void>();
      const setState = (next: Partial<LiveState>) => {
        state = { ...state, ...next };
        for (const listener of listeners) {
          listener();
        }
      };

      let socket: SocketLike | undefined;
      let destroyed = false;
      let attempt = 0;
      let retry: ReturnType<typeof setTimeout> | undefined;
      const waiting: { resolve: () => void; reject: (error: Error) => void }[] = [];
      const failDone = (error: Error) => {
        for (const entry of waiting.splice(0)) {
          entry.reject(error);
        }
      };

      let markReady!: () => void;
      const ready = new Promise<void>((resolve) => {
        markReady = resolve;
      });

      const send = (messages: readonly WireMessage[]) => {
        if (socket?.readyState === OPEN) {
          socket.send(encodeFrame(messages));
        }
      };
      const ownAwareness = (): WireMessage => ({
        type: "awareness",
        update: encodeAwarenessUpdate(awareness, [document_.clientID]),
      });

      const onCustom = (tag: number, payload: Uint8Array) => {
        if (tag === MSG_PEERS) {
          const entries = decodePeers(payload);
          const peers = new Map(
            entries.map((entry) => [
              entry.clientId,
              { user: entry.user, canWrite: entry.canWrite },
            ]),
          );
          // The server lists this editor once it has seen its awareness;
          // until then it stays read-only.
          setState({ peers, canWrite: peers.get(document_.clientID)?.canWrite ?? false });
        } else if (tag === MSG_DONE) {
          waiting.shift()?.resolve();
        }
        // Other custom tags are from newer servers; ignore them.
      };

      const onMessage = (message: WireMessage) => {
        switch (message.type) {
          case "sync-step-1": {
            send([
              {
                type: "sync-step-2",
                update: Y.encodeStateAsUpdate(document_, message.stateVector),
              },
            ]);
            break;
          }
          case "sync-step-2": {
            Y.applyUpdate(document_, message.update, REMOTE);
            if (state.status !== "online") {
              setState({ status: "online" });
              markReady();
            }
            break;
          }
          case "update": {
            Y.applyUpdate(document_, message.update, REMOTE);
            break;
          }
          case "awareness": {
            applyAwarenessUpdate(awareness, message.update, REMOTE);
            break;
          }
          case "awareness-query": {
            send([ownAwareness()]);
            break;
          }
          case "auth": {
            if (message.denied !== undefined) {
              revoke();
            }
            break;
          }
          case "custom": {
            onCustom(message.tag, message.payload);
            break;
          }
        }
      };

      const forgetPeers = () => {
        const others = [...awareness.getStates().keys()].filter((id) => id !== document_.clientID);
        removeAwarenessStates(awareness, others, REMOTE);
      };

      const revoke = () => {
        setState({ status: "revoked", canWrite: false });
        failDone(new Error("access revoked"));
        const current = socket;
        socket = undefined;
        current?.close(1000, "revoked");
        forgetPeers();
      };

      const connect = () => {
        retry = undefined;
        const current = socketFor(url);
        current.binaryType = "arraybuffer";
        socket = current;
        current.addEventListener("open", () => {
          attempt = 0;
          send([{ type: "sync-step-1", stateVector: Y.encodeStateVector(document_) }]);
          // Rewriting the presence state moves its clock on, and the update
          // handler sends it. Resending the same clock would not do: a fresh
          // state's clock 0, or the clock a room still holds from before a
          // drop, reads as already seen, and the server would never list
          // this editor as a writer.
          awareness.setLocalState(awareness.getLocalState() ?? {});
        });
        current.addEventListener("message", (event) => {
          if (!(event.data instanceof ArrayBuffer)) {
            return;
          }
          let messages: WireMessage[];
          try {
            messages = decodeFrame(new Uint8Array(event.data));
          } catch (error) {
            if (error instanceof WireError) {
              return;
            }
            throw error;
          }
          for (const message of messages) {
            try {
              onMessage(message);
            } catch (error) {
              // A malformed payload from the server drops that message only.
              if (!(error instanceof WireError)) {
                throw error;
              }
            }
          }
        });
        current.addEventListener("close", (event) => {
          if (socket !== current) {
            return;
          }
          socket = undefined;
          failDone(new Error("connection closed"));
          forgetPeers();
          if (destroyed) {
            return;
          }
          if (event.code === CLOSE_REVOKED) {
            setState({ status: "revoked", canWrite: false, peers: new Map() });
            return;
          }
          setState({
            status: state.status === "online" ? "offline" : state.status,
            peers: new Map(),
          });
          const ceiling = Math.min(RETRY_MAX_MS, RETRY_FIRST_MS * 2 ** attempt);
          attempt += 1;
          // Half to all of the ceiling, so editors dropped together do not
          // all come back in the same instant.
          retry = setTimeout(connect, ceiling * (0.5 + random() / 2));
        });
      };

      const onUpdate = (update: Uint8Array, from: unknown) => {
        if (from !== REMOTE) {
          send([{ type: "update", update }]);
        }
      };
      const onAwareness = (
        { added, updated, removed }: Record<"added" | "updated" | "removed", number[]>,
        from: unknown,
      ) => {
        if (from === REMOTE) {
          return;
        }
        const changed = [...added, ...updated, ...removed];
        send([{ type: "awareness", update: encodeAwarenessUpdate(awareness, changed) }]);
      };
      document_.on("update", onUpdate);
      awareness.on("update", onAwareness);
      connect();

      const live: LiveConnection = {
        getSnapshot: () => state,
        subscribe(listener) {
          listeners.add(listener);
          return () => {
            listeners.delete(listener);
          };
        },
        done() {
          if (state.status !== "online") {
            return Promise.reject(new Error("not connected"));
          }
          if (!state.canWrite) {
            return Promise.reject(new Error("read only"));
          }
          return new Promise<void>((resolve, reject) => {
            const timer = setTimeout(() => {
              const index = waiting.indexOf(entry);
              if (index !== -1) {
                waiting.splice(index, 1);
                reject(new Error("no answer from the server"));
              }
            }, doneTimeoutMs);
            const entry = {
              resolve: () => {
                clearTimeout(timer);
                resolve();
              },
              reject: (error: Error) => {
                clearTimeout(timer);
                reject(error);
              },
            };
            waiting.push(entry);
            send([{ type: "custom", tag: MSG_DONE, payload: new Uint8Array() }]);
          });
        },
      };

      return {
        doc: document_,
        awareness,
        ready,
        live,
        destroy() {
          destroyed = true;
          if (retry !== undefined) {
            clearTimeout(retry);
          }
          // Tell peers this caret is gone while the socket is still open.
          removeAwarenessStates(awareness, [document_.clientID], "destroy");
          document_.off("update", onUpdate);
          awareness.off("update", onAwareness);
          const current = socket;
          socket = undefined;
          current?.close(1000, "closed");
          failDone(new Error("closed"));
          listeners.clear();
          awareness.destroy();
          document_.destroy();
        },
      };
    },
  };
}
