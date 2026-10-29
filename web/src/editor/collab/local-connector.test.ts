// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, describe, expect, it, vi } from "vitest";
import * as Y from "yjs";

import { createLocalConnector, type ChannelFactory } from "./local-connector";
import type { CollabSession, UpdateCache } from "./provider";

/** An in-memory BroadcastChannel: delivers to the other channels of the same name. */
function createBus() {
  const rooms = new Map<string, Set<FakeChannel>>();
  const posted: unknown[] = [];

  class FakeChannel extends EventTarget {
    closed = false;

    constructor(readonly name: string) {
      super();
      const room = rooms.get(name) ?? new Set();
      room.add(this);
      rooms.set(name, room);
    }

    postMessage(data: unknown) {
      posted.push(data);
      for (const peer of rooms.get(this.name) ?? []) {
        if (peer !== this) {
          queueMicrotask(() => {
            peer.deliver(data);
          });
        }
      }
    }

    deliver(data: unknown) {
      this.dispatchEvent(new MessageEvent("message", { data }));
    }

    close() {
      this.closed = true;
      rooms.get(this.name)?.delete(this);
    }
  }

  const channels: FakeChannel[] = [];
  const factory: ChannelFactory = (name) => {
    const channel = new FakeChannel(name);
    channels.push(channel);
    return channel;
  };
  return { factory, channels, posted };
}

/** A cache that keeps updates in memory. */
function memoryCache(initial: Record<string, Uint8Array[]> = {}) {
  const stored = new Map(Object.entries(initial));
  const append = vi.fn((documentId: string, update: Uint8Array) => {
    stored.set(documentId, [...(stored.get(documentId) ?? []), update]);
    return Promise.resolve();
  });
  const cache: UpdateCache = {
    load: (documentId) => Promise.resolve([...(stored.get(documentId) ?? [])]),
    append,
  };
  return { cache, stored, append };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const text = (session: CollabSession) => session.doc.getText("t").toJSON();

const sessions: CollabSession[] = [];
function track(session: CollabSession) {
  sessions.push(session);
  return session;
}

afterEach(() => {
  for (const session of sessions.splice(0)) {
    session.destroy();
  }
});

describe("createLocalConnector", () => {
  it("syncs edits between two sessions of the same document", async () => {
    const bus = createBus();
    const connector = createLocalConnector({ channel: bus.factory });
    const first = track(connector.open("page-1"));
    const second = track(connector.open("page-1"));
    await Promise.all([first.ready, second.ready]);
    first.doc.getText("t").insert(0, "hello");
    await settle();
    expect(text(second)).toBe("hello");
    second.doc.getText("t").insert(5, " world");
    await settle();
    expect(text(first)).toBe("hello world");
  });

  it("catches a late joiner up through the hello exchange", async () => {
    const bus = createBus();
    const connector = createLocalConnector({ channel: bus.factory });
    const early = track(connector.open("page-1"));
    await early.ready;
    early.doc.getText("t").insert(0, "before you came");
    const late = track(connector.open("page-1"));
    await late.ready;
    await settle();
    expect(text(late)).toBe("before you came");
  });

  it("keeps different documents apart", async () => {
    const bus = createBus();
    const connector = createLocalConnector({ channel: bus.factory });
    const first = track(connector.open("page-1"));
    const other = track(connector.open("page-2"));
    await Promise.all([first.ready, other.ready]);
    first.doc.getText("t").insert(0, "private");
    await settle();
    expect(text(other)).toBe("");
  });

  it("shares awareness state and removes it on destroy", async () => {
    const bus = createBus();
    const connector = createLocalConnector({ channel: bus.factory });
    const first = track(connector.open("page-1"));
    const second = connector.open("page-1");
    await Promise.all([first.ready, second.ready]);
    second.awareness.setLocalStateField("user", { name: "Sam", color: "#123456" });
    await settle();
    expect(first.awareness.getStates().get(second.doc.clientID)).toEqual({
      user: { name: "Sam", color: "#123456" },
    });
    second.destroy();
    await settle();
    expect(first.awareness.getStates().has(second.doc.clientID)).toBe(false);
  });

  it("loads cached updates before announcing itself, and appends new ones", async () => {
    const source = new Y.Doc();
    source.getText("t").insert(0, "cached");
    const { cache, stored, append } = memoryCache({ "page-1": [Y.encodeStateAsUpdate(source)] });
    const bus = createBus();
    const session = track(createLocalConnector({ cache, channel: bus.factory }).open("page-1"));
    await session.ready;
    expect(text(session)).toBe("cached");
    expect(append).not.toHaveBeenCalled();
    expect(bus.posted.at(-1)).toMatchObject({ kind: "hello", from: session.doc.clientID });
    session.doc.getText("t").insert(0, "new ");
    expect(append).toHaveBeenCalledTimes(1);
    expect(stored.get("page-1")).toHaveLength(2);
  });

  it("caches remote updates without echoing them back", async () => {
    const { cache, append } = memoryCache();
    const bus = createBus();
    const writer = track(createLocalConnector({ channel: bus.factory }).open("page-1"));
    const reader = track(createLocalConnector({ cache, channel: bus.factory }).open("page-1"));
    await Promise.all([writer.ready, reader.ready]);
    await settle();
    const before = bus.posted.length;
    writer.doc.getText("t").insert(0, "x");
    await settle();
    expect(text(reader)).toBe("x");
    expect(append).toHaveBeenCalled();
    expect(bus.posted.slice(before)).toHaveLength(1);
  });

  it("ignores foreign messages and its own echoes", async () => {
    const bus = createBus();
    const session = track(createLocalConnector({ channel: bus.factory }).open("page-1"));
    await session.ready;
    const [channel] = bus.channels;
    for (const data of ["text", undefined, { kind: "update" }, 42]) {
      channel?.deliver(data);
    }
    const own = new Y.Doc();
    own.getText("t").insert(0, "echo");
    channel?.deliver({
      kind: "update",
      from: session.doc.clientID,
      update: Y.encodeStateAsUpdate(own),
    });
    expect(text(session)).toBe("");
  });

  it("closes its channel and stops listening on destroy", async () => {
    const bus = createBus();
    const session = createLocalConnector({ channel: bus.factory }).open("page-1");
    await session.ready;
    const [channel] = bus.channels;
    if (!channel) {
      throw new Error("the session opened no channel");
    }
    const removal = vi.spyOn(channel, "removeEventListener");
    session.destroy();
    expect(channel.closed).toBe(true);
    expect(removal).toHaveBeenCalledWith("message", expect.any(Function));
  });

  it("uses a real BroadcastChannel by default", async () => {
    const connector = createLocalConnector();
    const first = track(connector.open(`real-${String(Date.now())}`));
    await first.ready;
    first.doc.getText("t").insert(0, "over the wire");
    expect(text(first)).toBe("over the wire");
  });
});
