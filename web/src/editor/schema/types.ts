// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** An attribute declared in `schema/blocks.json`. */
export interface AttrData {
  readonly type: "string" | "integer" | "boolean" | "enum";
  readonly default: string | number | boolean;
  readonly values?: readonly string[];
  /** Editor-only state that is never saved. */
  readonly transient?: boolean;
}

/** A slash-menu entry. */
export interface SlashData {
  readonly title: string;
  readonly description: string;
  readonly keywords: readonly string[];
  readonly section: string;
  /** A Material Symbols icon name. */
  readonly icon: string;
  /** Attributes the inserted node starts with. */
  readonly attrs: Readonly<Record<string, string | number | boolean>>;
  /** Starter content, by template name (see `templates.ts`). */
  readonly template?: string;
}

/** A node type, as generated from the schema. */
export interface NodeData {
  readonly name: string;
  readonly doc: string;
  /** `tiptap`: a stock Tiptap extension; `generic`: built from this data; `custom`: hand written. */
  readonly impl: "tiptap" | "generic" | "custom";
  readonly group: string;
  readonly content: string;
  readonly marks?: string;
  readonly inline?: boolean;
  readonly atom?: boolean;
  readonly code?: boolean;
  readonly defining?: boolean;
  readonly isolating?: boolean;
  readonly draggable?: boolean;
  /** Whether the node carries a stable `blockId` attribute. */
  readonly blockId?: boolean;
  /** The Vellum Markdown construct the node maps to. */
  readonly markdown: string;
  readonly attrs: Readonly<Record<string, AttrData>>;
  readonly slash: readonly SlashData[];
}

/** A mark type, as generated from the schema. */
export interface MarkData {
  readonly name: string;
  readonly doc: string;
  readonly impl: "tiptap" | "generic" | "custom";
  readonly markdown: string;
  readonly attrs: Readonly<Record<string, AttrData>>;
  readonly excludes?: string;
}
