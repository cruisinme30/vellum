// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Attribute } from "@tiptap/core";

import { MARKS, NODES, type NodeName } from "./blocks.gen";
import type { AttributeData as AttributeData, NodeData } from "./types";

/** The generated data for one node type. */
export function nodeSpec(name: NodeName): NodeData {
  const spec = NODES.find((n) => n.name === name);
  if (!spec) {
    throw new Error(`Vellum: no node ${name} in the schema`);
  }
  return spec;
}

/** Node types that carry a `blockId` attribute. */
export const BLOCK_ID_TYPES: readonly string[] = NODES.filter((n: NodeData) => n.blockId).map(
  (n) => n.name,
);

/** Every node and mark name, for checks. */
export const SCHEMA_NAMES = {
  nodes: NODES.map((n) => n.name),
  marks: MARKS.map((m) => m.name),
};

const kebab = (name: string) => name.replaceAll(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);

type Value = string | number | boolean;

const PARSERS: Readonly<
  Record<AttributeData["type"], (raw: string, attribute: AttributeData) => Value>
> = {
  integer: (raw, attribute) => {
    const value = Number.parseInt(raw, 10);
    return Number.isFinite(value) ? value : attribute.default;
  },
  boolean: (raw) => raw === "true",
  enum: (raw, attribute) => (attribute.values?.includes(raw) ? raw : attribute.default),
  string: (raw) => raw,
};

/** Tiptap attribute definitions for a node, stored in the DOM as `data-*`. */
export function tiptapAttributes(spec: NodeData): Record<string, Attribute> {
  const out: Record<string, Attribute> = {};
  for (const [key, attribute] of Object.entries(spec.attrs)) {
    if (attribute.transient) {
      continue;
    }
    const dataName = `data-${kebab(key)}`;
    out[key] = {
      default: attribute.default,
      parseHTML: (element) => {
        const raw = element.getAttribute(dataName);
        return raw === null ? attribute.default : PARSERS[attribute.type](raw, attribute);
      },
      renderHTML: (attributes) => ({ [dataName]: String(attributes[key]) }),
    };
  }
  return out;
}
