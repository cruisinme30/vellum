// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Schema } from "@tiptap/pm/model";
import { describe, expect, it } from "vitest";
import * as Y from "yjs";

import { body, p } from "../content";

import { FIELD, seedUpdate } from "./seed";

const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    paragraph: { group: "block", content: "text*" },
    text: {},
  },
});

const CONTENT = body(p("Hello"), p("World"));

function applied(...updates: Uint8Array[]) {
  const document_ = new Y.Doc();
  for (const update of updates) {
    Y.applyUpdate(document_, update);
  }
  return document_;
}

describe("seedUpdate", () => {
  it("writes the content into the editor's fragment", () => {
    const fragment = applied(seedUpdate(schema, CONTENT)).getXmlFragment(FIELD);
    expect(fragment).toHaveLength(2);
    expect(fragment.toJSON()).toContain("Hello");
    expect(fragment.toJSON()).toContain("World");
  });

  it("is deterministic, so seeding twice never duplicates content", () => {
    const first = seedUpdate(schema, CONTENT);
    expect(seedUpdate(schema, CONTENT)).toEqual(first);
    const twice = applied(first, seedUpdate(schema, CONTENT));
    expect(twice.getXmlFragment(FIELD)).toHaveLength(2);
  });

  it(
    "keeps content a user deleted deleted when the seed is applied " + "again",
    () => {
      const seed = seedUpdate(schema, CONTENT);
      const document_ = applied(seed);
      document_.getXmlFragment(FIELD).delete(0, 1);
      Y.applyUpdate(document_, seed);
      expect(document_.getXmlFragment(FIELD)).toHaveLength(1);
      expect(document_.getXmlFragment(FIELD).toJSON()).not.toContain("Hello");
    },
  );
});
