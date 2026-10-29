// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Awareness, encodeAwarenessUpdate } from "y-protocols/awareness";
import * as Y from "yjs";

import { FakeRoom } from "./fake-room";
import type { CollabSession } from "./provider";
import { collabUrl, createSocketConnector } from "./socket-connector";
import { MSG_DONE, MSG_PEERS } from "./wire";

const text = (session: CollabSession) => session.doc.getText("t").toJSON();

let room: FakeRoom;
const sessions: CollabSession[] = [];
const open = (options: { random?: () => number; doneTimeoutMs?: number } = {}) => {
  const session = createSocketConnector({
    origin: "https://vellum.northstar.dev",
    socket: room.factory,
    random: () => 0.5,
    ...options,
  }).open("01JPAGE", "01JSPACE");
  sessions.push(session);
  return session;
};

beforeEach(() => {
  room = new FakeRoom();
});

afterEach(() => {
  vi.useRealTimers();
  for (const session of sessions.splice(0)) {
    session.destroy();
  }
});

describe("collabUrl", () => {
  it("joins the page's room on the app's origin", () => {
    expect(collabUrl("https://vellum.northstar.dev", "s1", "d1")).toBe(
      "wss://vellum.northstar.dev/collab/s1/d1",
    );
    expect(collabUrl("http://localhost:5173", "s1", "d1")).toBe("ws://localhost:5173/collab/s1/d1");
  });

  it("keeps IDs inside their path segments", () => {
    expect(collabUrl("https://a.dev", "../x", "y?z")).toBe("wss://a.dev/collab/..%2Fx/y%3Fz");
  });
});

describe("socket connector", () => {
  it("loads the room's content and goes online", async () => {
    room.doc.getText("t").insert(0, "from the server");
    const session = open();
    expect(session.live?.getSnapshot().status).toBe("connecting");
    await session.ready;
    expect(room.urls).toEqual(["wss://vellum.northstar.dev/collab/01JSPACE/01JPAGE"]);
    expect(room.last.binaryType).toBe("arraybuffer");
    expect(text(session)).toBe("from the server");
    expect(session.live?.getSnapshot().status).toBe("online");
  });

  it("sends local edits and applies other editors' edits", async () => {
    const session = open();
    await session.ready;
    session.doc.getText("t").insert(0, "mine");
    expect(room.doc.getText("t").toJSON()).toBe("mine");
    room.edit((document_) => {
      document_.getText("t").insert(4, " and theirs");
    });
    expect(text(session)).toBe("mine and theirs");
  });

  it("does not echo the room's edits back", async () => {
    const session = open();
    await session.ready;
    const before = room.received.filter((m) => m.type === "update").length;
    room.edit((document_) => {
      document_.getText("t").insert(0, "x");
    });
    expect(text(session)).toBe("x");
    expect(room.received.filter((m) => m.type === "update")).toHaveLength(before);
  });

  it("may write once the server lists this editor as a writer", async () => {
    const session = open();
    await session.ready;
    const state = session.live?.getSnapshot();
    expect(state?.canWrite).toBe(true);
    expect(state?.peers.get(session.doc.clientID)?.canWrite).toBe(true);
  });

  it("stays read-only for a viewer", async () => {
    room.canWrite = false;
    const session = open();
    await session.ready;
    expect(session.live?.getSnapshot().canWrite).toBe(false);
    await expect(session.live?.done()).rejects.toThrow("read only");
  });

  it("is read-only until the server has listed it", () => {
    const session = open();
    expect(session.live?.getSnapshot().canWrite).toBe(false);
  });

  it("asks for a version on Done and resolves when it is written", async () => {
    const session = open();
    await session.ready;
    await expect(session.live?.done()).resolves.toBeUndefined();
    expect(room.received).toContainEqual({
      type: "custom",
      tag: MSG_DONE,
      payload: new Uint8Array(),
    });
  });

  it("gives up on Done when the server does not answer", async () => {
    room.answerDone = false;
    const session = open({ doneTimeoutMs: 20 });
    await session.ready;
    await expect(session.live?.done()).rejects.toThrow("no answer");
  });

  it("refuses Done while not connected", async () => {
    room.autoOpen = false;
    const session = open();
    await expect(session.live?.done()).rejects.toThrow("not connected");
  });

  it("fails a pending Done when the socket drops", async () => {
    room.answerDone = false;
    const session = open();
    await session.ready;
    const pending = session.live?.done();
    room.last.drop();
    await expect(pending).rejects.toThrow("connection closed");
  });

  it("reconnects with backoff and sends edits made while offline", async () => {
    vi.useFakeTimers();
    const session = open();
    await vi.advanceTimersByTimeAsync(0);
    await session.ready;
    const statuses: string[] = [];
    session.live?.subscribe(() => statuses.push(session.live?.getSnapshot().status ?? ""));

    room.last.drop();
    expect(session.live?.getSnapshot().status).toBe("offline");
    session.doc.getText("t").insert(0, "written offline");
    expect(room.doc.getText("t").toJSON()).toBe("");

    // First retry: half to all of 500 ms; random 0.5 gives 375 ms.
    await vi.advanceTimersByTimeAsync(374);
    expect(room.sockets).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(room.sockets).toHaveLength(2);
    expect(room.doc.getText("t").toJSON()).toBe("written offline");
    expect(session.live?.getSnapshot().status).toBe("online");
    expect(statuses).toContain("offline");
  });

  it("backs off further after each failed attempt, up to 30 s", async () => {
    vi.useFakeTimers();
    room.autoOpen = false;
    open({ random: () => 0.999_999 });
    const delays: number[] = [];
    let last = 0;
    for (let index = 0; index < 9; index++) {
      const count = room.sockets.length;
      room.last.drop();
      let waited = 0;
      while (room.sockets.length === count) {
        await vi.advanceTimersByTimeAsync(100);
        waited += 100;
      }
      delays.push(waited);
      last = waited;
    }
    expect(delays.slice(0, 4)).toEqual([500, 1000, 2000, 4000]);
    expect(last).toBe(30_000);
  });

  it("announces its caret with a newer clock on every connection", async () => {
    vi.useFakeTimers();
    const session = open();
    await vi.advanceTimersByTimeAsync(0);
    await session.ready;
    const clock = () => room.awareness.meta.get(session.doc.clientID)?.clock ?? -1;
    const first = clock();
    expect(first).toBeGreaterThan(0);
    room.last.drop();
    await vi.advanceTimersByTimeAsync(1000);
    expect(room.sockets).toHaveLength(2);
    expect(clock()).toBeGreaterThan(first);
    expect(session.live?.getSnapshot().canWrite).toBe(true);
  });

  it("stops for good when access is revoked", async () => {
    vi.useFakeTimers();
    const session = open();
    await vi.advanceTimersByTimeAsync(0);
    await session.ready;
    room.last.drop(4403);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(room.sockets).toHaveLength(1);
    expect(session.live?.getSnapshot()).toMatchObject({ status: "revoked", canWrite: false });
  });

  it("forgets other editors' carets when the socket drops", async () => {
    const session = open();
    await session.ready;
    const other = new Awareness(new Y.Doc());
    other.setLocalState({ user: { name: "Lee" } });
    room.last.deliver([
      { type: "awareness", update: encodeAwarenessUpdate(other, [other.clientID]) },
    ]);
    expect(session.awareness.getStates().has(other.clientID)).toBe(true);
    room.last.drop();
    expect(session.awareness.getStates().has(other.clientID)).toBe(false);
    expect(session.awareness.getStates().has(session.doc.clientID)).toBe(true);
  });

  it("ignores malformed frames and text frames from the server", async () => {
    const session = open();
    await session.ready;
    room.last.deliverRaw(Uint8Array.from([0, 0, 9]).buffer);
    room.last.deliverRaw("hello");
    room.last.deliver([{ type: "custom", tag: MSG_PEERS, payload: Uint8Array.from([9]) }]);
    room.last.deliver([{ type: "custom", tag: 120, payload: Uint8Array.from([1]) }]);
    room.edit((document_) => {
      document_.getText("t").insert(0, "still fine");
    });
    expect(text(session)).toBe("still fine");
    expect(session.live?.getSnapshot().canWrite).toBe(true);
  });

  it("answers an awareness query with its own state", async () => {
    const session = open();
    await session.ready;
    const before = room.received.filter((m) => m.type === "awareness").length;
    room.last.deliver([{ type: "awareness-query" }]);
    expect(room.received.filter((m) => m.type === "awareness")).toHaveLength(before + 1);
  });

  it("treats an auth denial as revoked", async () => {
    const session = open();
    await session.ready;
    const socket = room.last;
    socket.deliver([{ type: "auth", denied: "gone" }]);
    expect(session.live?.getSnapshot().status).toBe("revoked");
    expect(socket.closedWith).toBe(1000);
  });

  it("closes the socket on destroy and never reconnects", async () => {
    vi.useFakeTimers();
    const session = open();
    await vi.advanceTimersByTimeAsync(0);
    await session.ready;
    const socket = room.last;
    const awarenessBefore = room.received.filter((m) => m.type === "awareness").length;
    sessions.splice(0);
    session.destroy();
    expect(socket.closedWith).toBe(1000);
    // The caret's removal went out before the close.
    expect(room.received.filter((m) => m.type === "awareness")).toHaveLength(awarenessBefore + 1);
    socket.drop(1000);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(room.sockets).toHaveLength(1);
  });

  it("cancels a pending retry on destroy", async () => {
    vi.useFakeTimers();
    room.autoOpen = false;
    const session = open();
    room.last.drop();
    sessions.splice(0);
    session.destroy();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(room.sockets).toHaveLength(1);
  });

  it("tells subscribers about changes until they unsubscribe", async () => {
    const session = open();
    const listener = vi.fn<() => void>();
    const unsubscribe = session.live?.subscribe(listener);
    await session.ready;
    expect(listener).toHaveBeenCalled();
    listener.mockClear();
    unsubscribe?.();
    room.last.drop();
    expect(listener).not.toHaveBeenCalled();
  });
});
