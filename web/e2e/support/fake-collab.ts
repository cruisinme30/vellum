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

/**
 * Stands in for the live-editing rooms of `vellum serve` (`/collab/...`,
 * crates/vellum-collab): it greets with sync step 1, answers step 1 with
 * step 2, applies edits from writers, lists each editor once its presence
 * arrives, and answers Done.
 */
export async function fakeCollab(
  page: Page,
  options: { canWrite?: boolean; paragraph?: string } = {},
): Promise<FakeCollab> {
  const canWrite = options.canWrite ?? true;
  const room = new Y.Doc();
  if (options.paragraph !== undefined) {
    const paragraph = new Y.XmlElement("paragraph");
    paragraph.insert(0, [new Y.XmlText(options.paragraph)]);
    room.getXmlFragment("default").insert(0, [paragraph]);
  }
  const urls: string[] = [];
  const clients = new Set<number>();
  let dones = 0;

  const peers = (): WireMessage => ({
    type: "custom",
    tag: MSG_PEERS,
    payload: encodePeers(
      [...clients].map((clientId) => ({ clientId, user: "01M6409GN2EYWS8Z7NFQQ3JNWZ", canWrite })),
    ),
  });

  await page.routeWebSocket("**/collab/**", (socket) => {
    urls.push(socket.url());
    const send = (messages: WireMessage[]) => {
      socket.send(Buffer.from(encodeFrame(messages)));
    };
    send([{ type: "sync-step-1", stateVector: Y.encodeStateVector(room) }, peers()]);
    socket.onMessage((data) => {
      if (typeof data === "string") {
        void socket.close({ code: 1003, reason: "binary frames only" });
        return;
      }
      for (const message of decodeFrame(new Uint8Array(data))) {
        switch (message.type) {
          case "sync-step-1": {
            send([
              { type: "sync-step-2", update: Y.encodeStateAsUpdate(room, message.stateVector) },
            ]);
            break;
          }
          case "sync-step-2":
          case "update": {
            if (canWrite) {
              Y.applyUpdate(room, message.update);
            }
            break;
          }
          case "awareness": {
            // The first number of an awareness update is its count, then
            // each entry starts with its client ID; one entry is enough here.
            const id = readClientId(message.update);
            if (id !== undefined && !clients.has(id)) {
              clients.add(id);
              send([peers()]);
            }
            break;
          }
          case "custom": {
            if (message.tag === MSG_DONE) {
              dones += 1;
              send([{ type: "custom", tag: MSG_DONE, payload: new Uint8Array() }]);
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

  return {
    urls,
    text: () =>
      room
        .getXmlFragment("default")
        .toArray()
        .map((block) => (block instanceof Y.XmlElement ? textOf(block) : ""))
        .join("\n"),
    dones: () => dones,
  };
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
