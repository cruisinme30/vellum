// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";
import { describe, expect, it, vi } from "vitest";

import { b, body, callout, link, mention, p, wiki } from "../editor/content";

import {
  createMemoryStore,
  mentionsHandle,
  newPageId,
  plainText,
  type WorkspaceSeed,
} from "./store";
import type { PageMeta } from "./types";

const SPACE = "space-a";
const NO_DOCUMENT = undefined;
const NOW = new Date("2026-10-29T12:00:00.000Z");

function page(
  id: string,
  title: string,
  parentId?: string,
  updatedAt = "2026-10-01",
): PageMeta {
  return {
    id,
    spaceId: SPACE,
    parentId,
    position: 1,
    title,
    icon: "📄",
    status: "draft",
    owner: "priya",
    updatedAt,
    starred: false,
  };
}

const LONG_PREFIX = "lorem ipsum ".repeat(10);
const LONG_SUFFIX = " dolor sit amet".repeat(10);

const SEED: WorkspaceSeed = {
  name: "Test",
  me: { handle: "priya", name: "Priya" },
  people: [{ handle: "priya", name: "Priya" }],
  spaces: [{ id: SPACE, slug: "a", name: "A", icon: "🅰" }],
  pages: [
    page("parent", "Gateway notes", undefined, "2026-10-01"),
    page("child", "Child", "parent", "2026-10-03"),
    page("other", "Budget", undefined, "2026-10-02"),
  ],
  bodies: {
    parent: body(p("Ask ", mention("dev"), " about the ", b("gateway"), ".")),
    child: body(
      p(`${LONG_PREFIX}needle${LONG_SUFFIX}`),
      p("cc ", mention("dev")),
    ),
    other: body(p("Mentions the gateway too")),
  },
};

/** Collapses runs of spaces: search text joins nodes with spaces. */
const squash = (text: string) => text.replaceAll(/\s+/g, " ");

const createStore = () => createMemoryStore(SEED, () => NOW);

describe("mentionsHandle", () => {
  it("finds a nested mention of the handle", () => {
    const document_ = body(callout("info", p("hi ", mention("sam"))));
    expect(mentionsHandle(document_, "sam")).toBe(true);
    expect(mentionsHandle(document_, "dev")).toBe(false);
  });

  it("is false for a missing document", () => {
    expect(mentionsHandle(NO_DOCUMENT, "sam")).toBe(false);
  });
});

describe("plainText", () => {
  it("joins text, mentions and atom attributes", () => {
    const document_ = body(
      p("See ", link("docs", "https://example.com"), " by ", mention("sam")),
      p(wiki("Target page", "Label")),
    );
    expect(squash(plainText(document_))).toBe(
      "See docs by @sam Target page Label",
    );
  });

  it(
    "is empty for a missing document and tolerates a mention without a " +
      "handle",
    () => {
      expect(plainText(NO_DOCUMENT)).toBe("");
      const bare: JSONContent = { type: "mention" };
      expect(plainText(bare)).toBe("@");
    },
  );
});

describe("newPageId", () => {
  it("makes a 26-character Crockford ULID from the time and randomness", () => {
    const id = newPageId(0, () => 0);
    expect(id).toBe("0".repeat(26));
    expect(newPageId(Date.UTC(2026, 0, 1), () => 0.999)).toMatch(
      /^[0-9A-HJKMNP-TV-Z]{10}Z{16}$/,
    );
  });

  it("uses the clock and Math.random by default", () => {
    expect(newPageId()).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
  });
});

describe("createMemoryStore", () => {
  it("exposes the seed as a snapshot", () => {
    const snapshot = createStore().getSnapshot();
    expect(snapshot.name).toBe("Test");
    expect(snapshot.pages).toHaveLength(3);
  });

  it("creates a page after its siblings and notifies listeners", async () => {
    const store = createStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    const created = await store.createPage({
      spaceId: SPACE,
      parentId: "parent",
      title: "New",
    });
    expect(created).toMatchObject({
      parentId: "parent",
      position: 2,
      title: "New",
      owner: "priya",
      updatedAt: NOW.toISOString(),
      starred: false,
    });
    expect(store.getSnapshot().pages).toContain(created);
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    await store.createPage({ spaceId: SPACE, parentId: undefined });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("creates the first page of an empty parent with an empty title", async () => {
    const created = await createStore().createPage({
      spaceId: SPACE,
      parentId: "child",
    });
    expect(created?.position).toBe(1);
    expect(created?.title).toBe("");
  });

  it("renames and stars a page", () => {
    const store = createStore();
    store.renamePage("other", "Budget 2027");
    store.toggleStar("other");
    const renamed = store
      .getSnapshot()
      .pages.find((entry) => entry.id === "other");
    expect(renamed).toMatchObject({
      title: "Budget 2027",
      starred: true,
      updatedAt: NOW.toISOString(),
    });
    store.toggleStar("other");
    expect(
      store.getSnapshot().pages.find((entry) => entry.id === "other")?.starred,
    ).toBe(false);
  });

  it("moves a page", () => {
    const store = createStore();
    store.movePage("other", { spaceId: SPACE, parentId: "parent", index: 0 });
    expect(
      store.getSnapshot().pages.find((entry) => entry.id === "other")?.parentId,
    ).toBe("parent");
  });

  it("trashes a page with its subtree", () => {
    const store = createStore();
    store.trashPage("parent");
    expect(store.getSnapshot().pages.map((entry) => entry.id)).toEqual([
      "other",
    ]);
  });

  it("never has a problem to dismiss", () => {
    const store = createStore();
    const before = store.getSnapshot();
    store.dismissProblem();
    expect(store.getSnapshot()).toBe(before);
  });

  it("serves seed bodies", () => {
    const store = createStore();
    expect(store.seedFor("parent")).toBe(SEED.bodies["parent"]);
    expect(store.seedFor("missing")).toBeUndefined();
  });

  it("uses the real clock by default", async () => {
    const created = await createMemoryStore(SEED).createPage({
      spaceId: SPACE,
      parentId: undefined,
    });
    expect(Number.isNaN(Date.parse(created?.updatedAt ?? ""))).toBe(false);
  });
});

describe("search", () => {
  it("returns nothing for a blank query", async () => {
    expect(await createStore().search("   ")).toEqual([]);
  });

  it("ranks title matches first, then by title", async () => {
    const hits = await createStore().search("Gateway");
    expect(hits.map((hit) => hit.page.id)).toEqual(["parent", "other"]);
    expect(hits[0]?.snippet).toContain("gateway");
    expect(hits[1]?.snippet).toBe("Mentions the gateway too");
  });

  it("matches titles without a body match", async () => {
    const hits = await createStore().search("budget");
    expect(hits.map((hit) => [hit.page.id, hit.snippet])).toEqual([
      ["other", ""],
    ]);
  });

  it("trims long snippets with ellipses on both sides", async () => {
    const [hit] = await createStore().search("needle");
    expect(hit?.snippet.startsWith("…")).toBe(true);
    expect(hit?.snippet.endsWith("…")).toBe(true);
    expect(hit?.snippet).toContain("needle");
  });

  it("sorts several title matches alphabetically", async () => {
    const store = createStore();
    store.renamePage("child", "A gateway child");
    const hits = await store.search("gateway");
    expect(hits.map((hit) => hit.page.id)).toEqual([
      "child",
      "parent",
      "other",
    ]);
  });
});

describe("mentions", () => {
  it("lists pages that mention a handle, newest first, with a snippet", async () => {
    const hits = await createStore().mentions("dev");
    expect(hits.map((hit) => hit.page.id)).toEqual(["child", "parent"]);
    expect(squash(hits[1]?.snippet ?? "")).toBe("Ask @dev about the gateway .");
  });

  it("is empty for a handle nobody mentions", async () => {
    expect(await createStore().mentions("nobody")).toEqual([]);
  });
});
