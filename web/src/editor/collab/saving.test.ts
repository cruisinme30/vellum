// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as Y from "yjs";

import type { LiveConnection, LiveState } from "./provider";
import { createSaver } from "./saving";

const IDLE = 1000;

/** A live connection whose state and Done answers the test controls. */
function fakeLive(initial: Partial<LiveState> = {}) {
  let state: LiveState = {
    status: "online",
    canWrite: true,
    peers: new Map(),
    ...initial,
  };
  const listeners = new Set<() => void>();
  const answers: { resolve: () => void; reject: (error: Error) => void }[] = [];
  const done = vi.fn(
    () =>
      new Promise<void>((resolve, reject) => {
        answers.push({ resolve, reject });
      }),
  );
  const live: LiveConnection = {
    getSnapshot: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    done,
  };
  return {
    live,
    done,
    answer: () => answers.shift()?.resolve(),
    fail: () => answers.shift()?.reject(new Error("no answer")),
    set(next: Partial<LiveState>) {
      state = { ...state, ...next };
      for (const listener of listeners) listener();
    },
  };
}

function type(document_: Y.Doc, text: string) {
  document_.getText("t").insert(0, text);
}

describe("saving", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("writes a version once the editor goes quiet", async () => {
    const document_ = new Y.Doc();
    const fake = fakeLive();
    const saver = createSaver(document_, fake.live, { idleMs: IDLE });
    expect(saver.getSnapshot()).toBe("saved");

    type(document_, "a");
    expect(saver.getSnapshot()).toBe("unsaved");
    await vi.advanceTimersByTimeAsync(IDLE / 2);
    type(document_, "b");
    await vi.advanceTimersByTimeAsync(IDLE / 2);
    expect(fake.done).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(IDLE / 2);
    expect(fake.done).toHaveBeenCalledTimes(1);
    expect(saver.getSnapshot()).toBe("saving");

    fake.answer();
    await vi.advanceTimersByTimeAsync(0);
    expect(saver.getSnapshot()).toBe("saved");
    saver.dispose();
  });

  it("ignores updates from the server and the seed", () => {
    const document_ = new Y.Doc();
    const fake = fakeLive();
    const saver = createSaver(document_, fake.live, { idleMs: IDLE });
    const other = new Y.Doc();
    type(other, "from a peer");
    Y.applyUpdate(document_, Y.encodeStateAsUpdate(other));
    expect(saver.getSnapshot()).toBe("saved");
    saver.dispose();
  });

  it("stays unsaved when edits arrive while a version is written", async () => {
    const document_ = new Y.Doc();
    const fake = fakeLive();
    const saver = createSaver(document_, fake.live, { idleMs: IDLE });
    type(document_, "a");
    void saver.save();
    expect(saver.getSnapshot()).toBe("saving");
    type(document_, "b");
    fake.answer();
    await vi.advanceTimersByTimeAsync(0);
    expect(saver.getSnapshot()).toBe("unsaved");
    await vi.advanceTimersByTimeAsync(IDLE);
    expect(fake.done).toHaveBeenCalledTimes(2);
    saver.dispose();
  });

  it("marks a failed save and retries after the next quiet spell", async () => {
    const document_ = new Y.Doc();
    const fake = fakeLive();
    const saver = createSaver(document_, fake.live, { idleMs: IDLE });
    type(document_, "a");
    void saver.save();
    fake.fail();
    await vi.advanceTimersByTimeAsync(0);
    expect(saver.getSnapshot()).toBe("failed");
    await vi.advanceTimersByTimeAsync(IDLE);
    fake.answer();
    await vi.advanceTimersByTimeAsync(0);
    expect(saver.getSnapshot()).toBe("saved");
    saver.dispose();
  });

  it("waits for the connection, then saves after reconnecting", async () => {
    const document_ = new Y.Doc();
    const fake = fakeLive({ status: "offline" });
    const saver = createSaver(document_, fake.live, { idleMs: IDLE });
    type(document_, "a");
    await vi.advanceTimersByTimeAsync(IDLE * 2);
    expect(fake.done).not.toHaveBeenCalled();
    expect(saver.getSnapshot()).toBe("unsaved");

    fake.set({ status: "online" });
    await vi.advanceTimersByTimeAsync(IDLE);
    expect(fake.done).toHaveBeenCalledTimes(1);
    saver.dispose();
  });

  it("asks the server on Done even with nothing new, but not for viewers", async () => {
    const document_ = new Y.Doc();
    const fake = fakeLive();
    const saver = createSaver(document_, fake.live, { idleMs: IDLE });
    void saver.save();
    expect(fake.done).toHaveBeenCalledTimes(1);
    fake.answer();
    await vi.advanceTimersByTimeAsync(0);
    expect(saver.getSnapshot()).toBe("saved");

    fake.set({ canWrite: false });
    await saver.save();
    expect(fake.done).toHaveBeenCalledTimes(1);
    saver.dispose();
  });

  it("stops after dispose", async () => {
    const document_ = new Y.Doc();
    const fake = fakeLive();
    const saver = createSaver(document_, fake.live, { idleMs: IDLE });
    type(document_, "a");
    saver.dispose();
    await vi.advanceTimersByTimeAsync(IDLE * 2);
    expect(fake.done).not.toHaveBeenCalled();
  });
});
