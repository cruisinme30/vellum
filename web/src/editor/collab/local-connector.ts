// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  applyAwarenessUpdate,
  Awareness,
  encodeAwarenessUpdate,
  removeAwarenessStates,
} from "y-protocols/awareness";
import * as Y from "yjs";

import type { CollabConnector, CollabSession, UpdateCache } from "./provider";

type Message =
  | { readonly kind: "hello"; readonly from: number; readonly vector: Uint8Array }
  | { readonly kind: "update"; readonly from: number; readonly update: Uint8Array }
  | { readonly kind: "awareness"; readonly from: number; readonly update: Uint8Array };

/** A broadcast channel, injectable for tests. */
export type ChannelFactory = (
  name: string,
) => Pick<BroadcastChannel, "postMessage" | "close" | "addEventListener" | "removeEventListener">;

const REMOTE = Symbol("remote");

const isMessage = (data: unknown): data is Message =>
  typeof data === "object" && data !== null && "kind" in data && "from" in data;

/**
 * Syncs a document between tabs of the same browser over BroadcastChannel,
 * with an optional durable cache. It speaks the same state-vector and
 * update exchange as the y-sync protocol, so swapping in the server is a
 * transport change only.
 */
export function createLocalConnector(
  options: { cache?: UpdateCache; channel?: ChannelFactory } = {},
): CollabConnector {
  const channelFor: ChannelFactory = options.channel ?? ((name) => new BroadcastChannel(name));

  return {
    open(documentId): CollabSession {
      const document_ = new Y.Doc();
      const awareness = new Awareness(document_);
      const channel = channelFor(`vellum-collab:${documentId}`);
      const send = (message: Message) => {
        channel.postMessage(message);
      };

      const onMessage = (event: MessageEvent<unknown>) => {
        const message = event.data;
        if (!isMessage(message) || message.from === document_.clientID) {
          return;
        }
        switch (message.kind) {
          case "hello": {
            send({
              kind: "update",
              from: document_.clientID,
              update: Y.encodeStateAsUpdate(document_, message.vector),
            });
            send({
              kind: "awareness",
              from: document_.clientID,
              update: encodeAwarenessUpdate(awareness, [document_.clientID]),
            });
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
        }
      };
      channel.addEventListener("message", onMessage);

      const onUpdate = (update: Uint8Array, origin: unknown) => {
        void options.cache?.append(documentId, update);
        if (origin !== REMOTE) {
          send({ kind: "update", from: document_.clientID, update });
        }
      };
      const onAwareness = (
        { added, updated, removed }: Record<"added" | "updated" | "removed", number[]>,
        origin: unknown,
      ) => {
        if (origin !== REMOTE) {
          const changed = [...added, ...updated, ...removed];
          send({
            kind: "awareness",
            from: document_.clientID,
            update: encodeAwarenessUpdate(awareness, changed),
          });
        }
      };

      const ready = (async () => {
        const stored = (await options.cache?.load(documentId)) ?? [];
        Y.transact(
          document_,
          () => {
            for (const update of stored) {
              Y.applyUpdate(document_, update, REMOTE);
            }
          },
          REMOTE,
        );
        document_.on("update", onUpdate);
        awareness.on("update", onAwareness);
        send({ kind: "hello", from: document_.clientID, vector: Y.encodeStateVector(document_) });
      })();

      return {
        doc: document_,
        awareness,
        ready,
        destroy() {
          removeAwarenessStates(awareness, [document_.clientID], "destroy");
          document_.off("update", onUpdate);
          awareness.off("update", onAwareness);
          channel.removeEventListener("message", onMessage);
          channel.close();
          awareness.destroy();
          document_.destroy();
        },
      };
    },
  };
}
