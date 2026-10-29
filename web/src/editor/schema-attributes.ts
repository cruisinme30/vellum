// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Extension, type Attribute } from "@tiptap/core";

import { NODES } from "./schema/blocks.gen";
import { tiptapAttributes } from "./schema/spec";

/** Attributes the stock Tiptap nodes already define (or that have their own extension). */
const NATIVE: Readonly<Record<string, readonly string[]>> = {
  heading: ["level"],
  orderedList: ["start"],
  taskItem: ["checked", "attributes"],
};

/** The schema attributes a stock Tiptap node lacks, such as `tight` or `attributes`. */
export function missingAttributes(): { types: string[]; attributes: Record<string, Attribute> }[] {
  return NODES.filter((node) => node.impl === "tiptap").flatMap((node) => {
    const native = new Set(NATIVE[node.name]);
    const attributes = Object.fromEntries(
      Object.entries(tiptapAttributes(node)).filter(([key]) => !native.has(key)),
    );
    return Object.keys(attributes).length === 0 ? [] : [{ types: [node.name], attributes }];
  });
}

/**
 * Adds the schema's extra attributes to the stock Tiptap nodes, so what the
 * server parses from Markdown (list tightness, `{...}` attribute spans)
 * survives a round trip through the editor.
 */
export const SchemaAttributes = Extension.create({
  name: "schemaAttributes",
  addGlobalAttributes: missingAttributes,
});
