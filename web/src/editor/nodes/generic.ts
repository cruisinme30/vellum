// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { mergeAttributes, Node } from "@tiptap/core";

import type { NodeName } from "../schema/blocks.gen";
import { nodeSpec, tiptapAttributes } from "../schema/spec";

/**
 * Builds a node straight from the schema: a `<div data-type>` holding its
 * content, styled by `vl-<name>`. Containers whose body is ordinary
 * Markdown (callouts, columns, stats, facts…) need nothing more.
 */
export function genericNode(name: NodeName, extra: Record<string, string> = {}) {
  const spec = nodeSpec(name);
  return Node.create({
    name: spec.name,
    ...(spec.group === "" ? {} : { group: spec.group }),
    ...(spec.content === "" ? {} : { content: spec.content }),
    ...(spec.isolating === true ? { isolating: true } : {}),
    ...(spec.defining === true ? { defining: true } : {}),
    addAttributes: () => tiptapAttributes(spec),
    parseHTML: () => [{ tag: `div[data-type="${spec.name}"]` }],
    renderHTML: ({ HTMLAttributes }) => [
      "div",
      mergeAttributes(HTMLAttributes, extra, {
        "data-type": spec.name,
        class: `vl-block vl-${spec.name}`,
      }),
      0,
    ],
  });
}

/** Callouts are notes for assistive technology. */
export const Callout = genericNode("callout", { role: "note" });
export const Columns = genericNode("columns");
export const Column = genericNode("column");
export const Stats = genericNode("stats");
export const Facts = genericNode("facts");
export const Progress = genericNode("progress");
export const Cards = genericNode("cards");
export const Steps = genericNode("steps");
export const Pages = genericNode("pages");
/** Any other `::: name` container, kept verbatim. */
export const Container = genericNode("container");
