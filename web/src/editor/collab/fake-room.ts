// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Test support: a stand-in for the server's live-editing room.

import { applyAwarenessUpdate, Awareness } from "y-protocols/awareness";
import * as Y from "yjs";

import type { SocketLike } from "./socket-connector";
import {
  decodeFrame,
  encodeFrame,
  encodePeers,
  MSG_DONE,
  MSG_PEERS,
  type WireMessage,
} from "./wire";

const FROM_CLIENT = Symbol("client");

/**
 * A stand-in for a vellum-collab room (crates/vellum-collab/src/room.rs):
 * it greets with sync step 1 and the peer list, answers step 1 with step
 * 2, applies writes from editors that may write, lists an editor once its
 * awareness arrives, and answers Done.
 */
export class FakeRoom {
  readonly doc = new Y.Doc();
  readonly awareness = new Awareness(this.doc);
  readonly sockets: FakeSocket[] = [];
  readonly urls: string[] = [];
  readonly received: WireMessage[] = [];
  canWrite = true;
  answerDone = true;
  autoOpen = true;
  clients = new Map<number, FakeSocket>();

  constructor() {
    // The room keeps no awareness of its own.
    // eslint-disable-next-line unicorn/no-null -- y-protocols removes a state with null
    this.awareness.setLocalState(null);
  }

  factory = (url: string): SocketLike => {
    const socket = new FakeSocket(this);
    this.urls.push(url);
    this.sockets.push(socket);
    if (this.autoOpen) {
      queueMicrotask(() => {
        socket.accept();
      });
    }
    return socket;
  };

  get last(): FakeSocket {
    const socket = this.sockets.at(-1);
    if (!socket) {
      throw new Error("no socket");
    }
    return socket;
  }

  peers(): WireMessage {
    const entries = [...this.clients.keys()].map((clientId) => ({
      clientId,
      user: "01J00000000000000000000000",
      canWrite: this.canWrite,
    }));
    return { type: "custom", tag: MSG_PEERS, payload: encodePeers(entries) };
  }

  greet(socket: FakeSocket) {
    socket.deliver([
      { type: "sync-step-1", stateVector: Y.encodeStateVector(this.doc) },
      this.peers(),
    ]);
  }

  receive(socket: FakeSocket, frame: Uint8Array) {
    for (const message of decodeFrame(frame)) {
      this.received.push(message);
      switch (message.type) {
        case "sync-step-1": {
          socket.deliver([
            {
              type: "sync-step-2",
              update: Y.encodeStateAsUpdate(this.doc, message.stateVector),
            },
          ]);
          break;
        }
        case "sync-step-2":
        case "update": {
          if (this.canWrite) {
            Y.applyUpdate(this.doc, message.update, FROM_CLIENT);
          }
          break;
        }
        case "awareness": {
          applyAwarenessUpdate(this.awareness, message.update, FROM_CLIENT);
          for (const id of this.awareness.getStates().keys()) {
            this.clients.set(id, socket);
          }
          socket.deliver([this.peers()]);
          break;
        }
        case "custom": {
          if (message.tag === MSG_DONE && this.answerDone) {
            socket.deliver([
              { type: "custom", tag: MSG_DONE, payload: new Uint8Array() },
            ]);
          }
          break;
        }
        default: {
          break;
        }
      }
    }
  }

  /** Sends an edit made by another editor to every open socket. */
  edit(change: (document_: Y.Doc) => void) {
    const other = new Y.Doc();
    Y.applyUpdate(other, Y.encodeStateAsUpdate(this.doc));
    other.on("update", (update: Uint8Array) => {
      Y.applyUpdate(this.doc, update);
      for (const socket of this.sockets) {
        socket.deliver([{ type: "update", update }]);
      }
    });
    change(other);
  }
}

export class FakeSocket extends EventTarget implements SocketLike {
  binaryType: BinaryType = "blob";
  readyState = 0;
  closedWith: number | undefined;

  constructor(private readonly room: FakeRoom) {
    super();
  }

  override addEventListener<K extends "open" | "message" | "close">(
    type: K,
    listener: (event: WebSocketEventMap[K]) => void,
  ): void {
    super.addEventListener(type, listener as EventListener);
  }

  accept() {
    if (this.readyState !== 0) {
      return;
    }
    this.readyState = 1;
    this.dispatchEvent(new Event("open"));
    this.room.greet(this);
  }

  deliver(messages: readonly WireMessage[]) {
    this.deliverRaw(encodeFrame(messages).buffer);
  }

  deliverRaw(data: unknown) {
    if (this.readyState === 1) {
      this.dispatchEvent(new MessageEvent("message", { data }));
    }
  }

  send(data: Uint8Array) {
    if (this.readyState !== 1) {
      throw new Error("send on a socket that is not open");
    }
    this.room.receive(this, data);
  }

  close(code = 1000) {
    this.closedWith = code;
    this.readyState = 3;
  }

  /** The server or network ends the connection. */
  drop(code = 1006) {
    this.readyState = 3;
    this.dispatchEvent(new CloseEvent("close", { code }));
  }
}
