// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";
import { describe, expect, it } from "vitest";

import { isBlockId } from "../../editor/block-id";
import { buildTree } from "../tree";

import { createNorthstarStore, NORTHSTAR, NORTHSTAR_PEOPLE, PAGE } from "./northstar";

const alphabetical = (a: string, b: string) => a.localeCompare(b);

function blockIds(node: JSONContent): unknown[] {
  const own: unknown = node.attrs?.["blockId"];
  return [
    ...(own === undefined ? [] : [own]),
    ...(node.content ?? []).flatMap((child) => blockIds(child)),
  ];
}

describe("the Northstar demo", () => {
  it("has a body for every page", () => {
    for (const entry of NORTHSTAR.pages) {
      expect(NORTHSTAR.bodies[entry.id], entry.title).toBeDefined();
    }
    expect(Object.keys(NORTHSTAR.bodies).toSorted(alphabetical)).toEqual(
      Object.values(PAGE).toSorted(alphabetical),
    );
  });

  it("gives every block a unique, canonical block ID", () => {
    for (const [id, document_] of Object.entries(NORTHSTAR.bodies)) {
      const found = blockIds(document_);
      expect(found.length, id).toBeGreaterThan(0);
      expect(
        found.every((value) => isBlockId(value)),
        id,
      ).toBe(true);
      expect(new Set(found).size, id).toBe(found.length);
    }
  });

  it("only mentions people of the workspace", () => {
    const handles = new Set(NORTHSTAR_PEOPLE.map((person) => person.handle));
    const store = createNorthstarStore();
    for (const handle of ["priya", "dev", "alice", "sam"]) {
      expect(handles.has(handle)).toBe(true);
    }
    expect(store.mentions("sam").map((hit) => hit.page.id)).toEqual(
      expect.arrayContaining([PAGE.onCall, PAGE.onboarding]),
    );
  });

  it("builds a tree for each space with the gateway under engineering home", () => {
    const store = createNorthstarStore();
    const snapshot = store.getSnapshot();
    expect(snapshot.name).toBe("Northstar");
    expect(snapshot.spaces).toHaveLength(3);
    const [engineering] = snapshot.spaces;
    const tree = buildTree(snapshot.pages, engineering?.id ?? "");
    const home = tree.find((node) => node.page.id === PAGE.engHome);
    expect(home?.children.map((node) => node.page.id)).toEqual([PAGE.gateway, PAGE.onCall]);
  });

  it("finds pages by body text", () => {
    const hits = createNorthstarStore().search("token buckets");
    expect(hits.map((hit) => hit.page.id)).toEqual([PAGE.rateLimits]);
  });

  it("returns fresh, independent stores", () => {
    const first = createNorthstarStore();
    const second = createNorthstarStore();
    first.trashPage(PAGE.engHome);
    expect(second.getSnapshot().pages).toHaveLength(NORTHSTAR.pages.length);
  });
});
