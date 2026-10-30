// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";
import * as Y from "yjs";

import {
  decodeFrame,
  encodeFrame,
  encodePeers,
  MSG_DONE,
  MSG_PEERS,
  type WireMessage,
} from "../../src/editor/collab/wire";

/** What a test can see of the faked room. */
export interface FakeCollab {
  /** The socket URLs the app opened. */
  readonly urls: string[];
  /** The room's page body as plain text. */
  text(): string;
  /** How many Done requests reached the room. */
  dones(): number;
}

/** Who a browser joins the room as. */
interface Joiner {
  /** The user ID the server vouches for in the peer list. */
  readonly user?: string;
  readonly canWrite?: boolean;
}

/** A room several browsers can join, for two-browser tests (#98). */
export interface FakeRoom extends FakeCollab {
  /** Routes this page's collab socket into the room. */
  join(page: Page, joiner?: Joiner): Promise<void>;
}

const DEFAULT_USER = "01M6409GN2EYWS8Z7NFQQ3JNWZ";

/**
 * Stands in for the live-editing rooms of `vellum serve` (`/collab/...`,
 * crates/vellum-collab): it greets with sync step 1, answers step 1 with
 * step 2, applies edits from writers and passes them on to the other
 * sockets, relays presence, lists each editor once its presence arrives,
 * and answers Done.
 */
export function fakeRoom(
  options: {
    paragraph?: string;
    fences?: readonly { lang: string; source: string }[];
  } = {},
): FakeRoom {
  const room = new Y.Doc();
  if (options.paragraph !== undefined) {
    const paragraph = new Y.XmlElement("paragraph");
    paragraph.insert(0, [new Y.XmlText(options.paragraph)]);
    room.getXmlFragment("default").insert(0, [paragraph]);
  }
  for (const { lang, source } of options.fences ?? []) {
    const fence = new Y.XmlElement("dataFence");
    fence.setAttribute("lang", lang);
    fence.setAttribute("args", "");
    fence.insert(0, [new Y.XmlText(source)]);
    const body = room.getXmlFragment("default");
    body.insert(body.length, [fence]);
  }
  const urls: string[] = [];
  const clients = new Map<number, { user: string; canWrite: boolean }>();
  const sockets = new Set<{ send: (messages: WireMessage[]) => void }>();
  // The latest presence of each socket, replayed to whoever joins later.
  const presence = new Map<object, WireMessage>();
  let dones = 0;

  const peers = (): WireMessage => ({
    type: "custom",
    tag: MSG_PEERS,
    payload: encodePeers(
      [...clients].map(([clientId, { user, canWrite }]) => ({
        clientId,
        user,
        canWrite,
      })),
    ),
  });

  return {
    urls,
    text: () =>
      room
        .getXmlFragment("default")
        .toArray()
        .map((block) => (block instanceof Y.XmlElement ? textOf(block) : ""))
        .join("\n"),
    dones: () => dones,
    async join(page, joiner = {}) {
      const user = joiner.user ?? DEFAULT_USER;
      const canWrite = joiner.canWrite ?? true;
      await page.routeWebSocket("**/collab/**", (socket) => {
        urls.push(socket.url());
        const self = {
          send: (messages: WireMessage[]) => {
            socket.send(Buffer.from(encodeFrame(messages)));
          },
        };
        const others = (messages: WireMessage[]) => {
          for (const other of sockets) if (other !== self) other.send(messages);
        };
        const mine = new Set<number>();
        sockets.add(self);
        socket.onClose(() => {
          sockets.delete(self);
          presence.delete(self);
          for (const id of mine) clients.delete(id);
          others([peers()]);
        });
        self.send([
          { type: "sync-step-1", stateVector: Y.encodeStateVector(room) },
          peers(),
          ...presence.values(),
        ]);
        socket.onMessage((data) => {
          if (typeof data === "string") {
            void socket.close({ code: 1003, reason: "binary frames only" });
            return;
          }
          for (const message of decodeFrame(new Uint8Array(data))) {
            switch (message.type) {
              case "sync-step-1": {
                self.send([
                  {
                    type: "sync-step-2",
                    update: Y.encodeStateAsUpdate(room, message.stateVector),
                  },
                ]);
                break;
              }
              case "sync-step-2":
              case "update": {
                if (canWrite) {
                  Y.applyUpdate(room, message.update);
                  others([{ type: "update", update: message.update }]);
                }
                break;
              }
              case "awareness": {
                // The first number of an awareness update is its count, then
                // each entry starts with its client ID; one entry is enough
                // here.
                const id = readClientId(message.update);
                if (id !== undefined && !clients.has(id)) {
                  clients.set(id, { user, canWrite });
                  mine.add(id);
                  self.send([peers()]);
                  others([peers()]);
                }
                presence.set(self, message);
                others([message]);
                break;
              }
              case "custom": {
                if (message.tag === MSG_DONE) {
                  dones += 1;
                  self.send([
                    {
                      type: "custom",
                      tag: MSG_DONE,
                      payload: new Uint8Array(),
                    },
                  ]);
                }
                break;
              }
              default: {
                break;
              }
            }
          }
        });
      });
    },
  };
}

/** One browser in a room of its own; see {@link fakeRoom}. */
export async function fakeCollab(
  page: Page,
  options: {
    canWrite?: boolean;
    paragraph?: string;
    fences?: readonly { lang: string; source: string }[];
  } = {},
): Promise<FakeCollab> {
  const room = fakeRoom(options);
  await room.join(page, {
    ...(options.canWrite === undefined ? {} : { canWrite: options.canWrite }),
  });
  return room;
}

/** The text inside a block, without its marks. */
function textOf(element: Y.XmlElement): string {
  return element
    .toArray()
    .map((child) => (child instanceof Y.XmlText ? child.toJSON() : ""))
    .join("");
}

/** The client ID of the first entry of an awareness update. */
function readClientId(update: Uint8Array): number | undefined {
  let offset = 0;
  const uint = () => {
    let value = 0;
    let scale = 1;
    for (;;) {
      const byte = update[offset++];
      if (byte === undefined) {
        return;
      }
      value += (byte & 0x7f) * scale;
      if (byte < 0x80) {
        return value;
      }
      scale *= 0x80;
    }
  };
  const count = uint();
  return count === undefined || count === 0 ? undefined : uint();
}
