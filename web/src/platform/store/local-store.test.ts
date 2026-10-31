// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import "fake-indexeddb/auto";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";

import { setTelemetrySink, type TelemetrySink } from "../telemetry";

import { LocalStore, type LocalStoreOptions } from "./local-store";
import { type LocalDatabase, openLocalDatabase } from "./persistence";
import { FakeServer, freshDatabaseName, issueMutators } from "./test-support";
import { SyncError } from "./transport";

const recorded: { name: string; value?: number }[] = [];
const sink: TelemetrySink = {
  histogram(name, value) {
    recorded.push({ name, value });
  },
  count(name) {
    recorded.push({ name });
  },
};

async function setup(extra: Partial<LocalStoreOptions> = {}) {
  const database = await openLocalDatabase(freshDatabaseName());
  const server = new FakeServer();
  const store = await LocalStore.open({
    database,
    mutators: issueMutators,
    transport: server,
    ...extra,
  });
  return { database, server, store };
}

async function reopen(database: LocalDatabase, server: FakeServer) {
  return LocalStore.open({
    database,
    mutators: issueMutators,
    transport: server,
  });
}

describe("LocalStore", () => {
  const previous = setTelemetrySink(sink);
  afterEach(() => {
    recorded.length = 0;
  });
  afterAll(() => setTelemetrySink(previous));

  it(
    "applies a mutation synchronously and notifies subscribers " + "once",
    async () => {
      const { store } = await setup();
      const listener = vi.fn();
      const unsubscribe = store.subscribe(["issues", "other"], listener);
      const before = store.version;
      const mutation = store.mutate("createIssue", {
        key: "ENG-1",
        title: "Fix login",
      });
      expect(mutation.id).toBe(1);
      expect(store.get("issues", "ENG-1")).toEqual({
        title: "Fix login",
        state: "backlog",
      });
      expect(store.scan("issues")).toHaveLength(1);
      expect(listener).toHaveBeenCalledTimes(1);
      expect(store.version).toBe(before + 1);
      expect(
        recorded.some(
          (entry) => entry.name === "vellum.client.mutation.perceived_ms",
        ),
      ).toBe(true);
      unsubscribe();
      store.mutate("createIssue", { key: "ENG-2", title: "b" });
      expect(listener).toHaveBeenCalledTimes(1);
    },
  );

  it("keeps pending mutations and the client ID across a restart", async () => {
    const { database, server, store } = await setup();
    store.mutate("createIssue", { key: "ENG-1", title: "offline" });
    store.mutate("setState", { key: "ENG-1", state: "started" });
    await store.flushed();
    const again = await reopen(database, server);
    expect(again.clientId).toBe(store.clientId);
    expect(again.pending.map((mutation) => mutation.id)).toEqual([1, 2]);
    expect(again.get("issues", "ENG-1")).toEqual({
      title: "offline",
      state: "started",
    });
    expect(again.mutate("deleteIssue", { key: "ENG-1" }).id).toBe(3);
  });

  it(
    "pushes, pulls and confirms, then continues mutation IDs after a " +
      "restart",
    async () => {
      const { database, server, store } = await setup();
      store.mutate("createIssue", { key: "ENG-1", title: "a" });
      const outcome = await store.sync();
      expect(outcome).toEqual({ pushed: 1, pulled: 1, pending: 0 });
      expect(store.get("issues", "ENG-1")).toEqual({
        title: "a",
        state: "backlog",
      });
      const again = await reopen(database, server);
      expect(again.pending).toEqual([]);
      expect(again.get("issues", "ENG-1")).toEqual({
        title: "a",
        state: "backlog",
      });
      expect(again.mutate("setState", { key: "ENG-1", state: "done" }).id).toBe(
        2,
      );
      expect(await again.sync()).toEqual({ pushed: 1, pulled: 1, pending: 0 });
    },
  );

  it("merges other people's changes under local pending edits", async () => {
    const { server, store } = await setup();
    server.serverApply({
      name: "createIssue",
      args: { key: "ENG-7", title: "theirs" },
    });
    await store.sync();
    store.mutate("setState", { key: "ENG-7", state: "started" });
    server.serverApply({
      name: "createIssue",
      args: { key: "ENG-8", title: "new" },
    });
    server.failNext = new SyncError("down", 503, true, "r");
    await expect(store.sync()).rejects.toBeInstanceOf(SyncError);
    expect(store.pending).toHaveLength(1);
    expect(store.get("issues", "ENG-7")).toMatchObject({ state: "started" });
    await store.sync();
    expect(store.get("issues", "ENG-8")).toMatchObject({ title: "new" });
    expect(store.get("issues", "ENG-7")).toEqual({
      title: "theirs",
      state: "started",
    });
    expect(store.pending).toEqual([]);
  });

  it("pulls page after page until the server has nothing more", async () => {
    const { server, store } = await setup();
    server.pageSize = 2;
    for (const key of ["A", "B", "C", "D", "E"]) {
      server.serverApply({ name: "createIssue", args: { key, title: key } });
    }
    expect(await store.sync()).toEqual({ pushed: 0, pulled: 5, pending: 0 });
    expect(store.scan("issues")).toHaveLength(5);
  });

  it("does not apply a mutation twice when a push is retried", async () => {
    const { server, store } = await setup();
    store.mutate("createIssue", { key: "ENG-1", title: "a" });
    await server.push({ clientId: store.clientId, mutations: store.pending });
    await store.sync();
    expect(server.log).toHaveLength(1);
  });

  it("drops and reports rejected mutations", async () => {
    const onRejected = vi.fn();
    const { database, server, store } = await setup({ onRejected });
    server.forbidden.add("deleteIssue");
    store.mutate("createIssue", { key: "ENG-1", title: "a" });
    store.mutate("deleteIssue", { key: "ENG-1" });
    expect(store.get("issues", "ENG-1")).toBeUndefined();
    await store.sync();
    expect(onRejected).toHaveBeenCalledWith(
      expect.objectContaining({ id: 2, code: "forbidden" }),
      expect.objectContaining({ name: "deleteIssue" }),
    );
    expect(store.get("issues", "ENG-1")).toMatchObject({ title: "a" });
    const reopened = await reopen(database, server);
    expect(reopened.pending).toEqual([]);
  });

  it("shares one round trip between concurrent sync calls", async () => {
    const { server, store } = await setup();
    store.mutate("createIssue", { key: "ENG-1", title: "a" });
    const [first, second] = await Promise.all([store.sync(), store.sync()]);
    expect(first).toBe(second);
    expect(server.pushes).toBe(1);
    await store.sync();
    expect(server.pushes).toBe(1);
  });

  it("refuses unknown mutators without queueing anything", async () => {
    const { store } = await setup();
    expect(() => store.mutate("nope", {})).toThrow("no mutator");
    expect(store.pending).toEqual([]);
  });

  it("reports disk failures and keeps working", async () => {
    const onDiskError = vi.fn();
    const { database, store } = await setup({ onDiskError });
    database.close();
    store.mutate("createIssue", { key: "ENG-1", title: "a" });
    await store.flushed();
    expect(onDiskError).toHaveBeenCalledTimes(1);
    expect(store.get("issues", "ENG-1")).toMatchObject({ title: "a" });
  });

  it("uses the injected clock for creation times", async () => {
    const { store } = await setup({ now: () => 42 });
    expect(
      store.mutate("createIssue", { key: "ENG-1", title: "a" }).createdAt,
    ).toBe(42);
  });
});
