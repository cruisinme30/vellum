// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { issueMutators } from "./test-support";
import type { JsonValue, Mutators } from "./types";
import { UnknownMutatorError, View } from "./view";

// The lint rules forbid the null literal, but null is a JSON value.
const NULL = JSON.parse("null") as JsonValue;

const create = (key: string, title = key) => ({
  name: "createIssue",
  args: { key, title },
});

describe("View", () => {
  it("shows a mutation before the server confirms it", () => {
    const view = new View(issueMutators);
    const touched = view.apply(create("ENG-1", "Fix login"));
    expect(touched).toEqual(new Set(["issues"]));
    expect(view.get("issues", "ENG-1")).toEqual({
      title: "Fix login",
      state: "backlog",
    });
  });

  it("scans base and overlay merged, sorted by key, without deleted rows", () => {
    const view = new View(issueMutators);
    view.load([
      { op: "put", collection: "issues", key: "ENG-3", value: { title: "c" } },
      { op: "put", collection: "issues", key: "ENG-1", value: { title: "a" } },
    ]);
    view.apply(create("ENG-2"));
    view.apply({ name: "deleteIssue", args: { key: "ENG-3" } });
    expect(view.scan("issues").map(([key]) => key)).toEqual(["ENG-1", "ENG-2"]);
    expect(view.get("issues", "ENG-3")).toBeUndefined();
    expect(view.scan("missing")).toEqual([]);
  });

  it("keeps null values distinct from missing ones", () => {
    const mutators: Mutators = {
      clear(tx) {
        tx.put("flags", "a", NULL);
      },
    };
    const view = new View(mutators);
    view.load([{ op: "put", collection: "flags", key: "a", value: true }]);
    view.apply({ name: "clear", args: {} });
    expect(view.get("flags", "a")).toBeNull();
  });

  it("lets a mutator read its own writes, including deletes", () => {
    const seen: unknown[] = [];
    const mutators: Mutators = {
      twice(tx) {
        tx.put("t", "b", 1);
        tx.put("t", "a", 2);
        seen.push(tx.get("t", "a"), tx.scan("t"));
        tx.del("t", "a");
        seen.push(tx.get("t", "a"), tx.scan("t"), tx.scan("other"));
      },
    };
    new View(mutators).apply({ name: "twice", args: {} });
    expect(seen).toEqual([
      2,
      [
        ["a", 2],
        ["b", 1],
      ],
      undefined,
      [["b", 1]],
      [],
    ]);
  });

  it("leaves the view unchanged when a mutator throws", () => {
    const view = new View(issueMutators);
    expect(() =>
      view.apply({ name: "setState", args: { key: "ENG-9", state: "done" } }),
    ).toThrow("no issue ENG-9");
    expect(view.scan("issues")).toEqual([]);
  });

  it("refuses unknown mutators", () => {
    const view = new View(issueMutators);
    expect(() => view.apply({ name: "nope", args: {} })).toThrow(
      UnknownMutatorError,
    );
  });

  it("rebases pending mutations onto new server state", () => {
    const view = new View(issueMutators);
    view.load([
      {
        op: "put",
        collection: "issues",
        key: "ENG-1",
        value: { title: "a", state: "backlog" },
      },
    ]);
    const pending = [
      {
        id: 2,
        name: "setState",
        args: { key: "ENG-1", state: "started" },
        createdAt: 0,
      },
    ];
    view.apply(pending[0] ?? create("x"));
    // Someone else renamed the issue meanwhile; both changes survive.
    const touched = view.rebase(
      [
        {
          op: "put",
          collection: "issues",
          key: "ENG-1",
          value: { title: "renamed", state: "backlog" },
        },
      ],
      false,
      pending,
    );
    expect(touched.has("issues")).toBe(true);
    expect(view.get("issues", "ENG-1")).toEqual({
      title: "renamed",
      state: "started",
    });
  });

  it("skips replays that now fail and honours resets", () => {
    const view = new View(issueMutators);
    view.load([
      {
        op: "put",
        collection: "issues",
        key: "ENG-1",
        value: { title: "a", state: "x" },
      },
    ]);
    view.load([{ op: "put", collection: "other", key: "k", value: 1 }]);
    const touched = view.rebase(
      [{ op: "del", collection: "issues", key: "ENG-1" }],
      true,
      [
        {
          id: 1,
          name: "setState",
          args: { key: "ENG-1", state: "done" },
          createdAt: 0,
        },
      ],
    );
    expect(touched).toEqual(new Set(["issues", "other"]));
    expect(view.scan("issues")).toEqual([]);
    expect(view.get("other", "k")).toBeUndefined();
  });
});
