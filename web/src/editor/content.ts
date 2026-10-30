// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";

import { hashedBlockId } from "./block-id";
import { NODES } from "./schema/blocks.gen";

/**
 * Small builders for demo documents. Blocks get stable IDs derived from the
 * page and their order, so every browser seeds the same collab document.
 */

type Inline = JSONContent | string;
type Mark = NonNullable<JSONContent["marks"]>[number];

const WITH_IDS = new Set<string>(
  NODES.filter((n) => "blockId" in n).map((n) => n.name),
);

/** Adds a block ID to every node that carries one. */
export function withIds(pageId: string, document_: JSONContent): JSONContent {
  let counter = 0;
  const visit = (node: JSONContent): JSONContent => {
    const content = node.content?.map((child) => visit(child));
    const type = node.type ?? "";
    let attributes = node.attrs;
    if (WITH_IDS.has(type)) {
      counter += 1;
      attributes = {
        ...node.attrs,
        blockId: hashedBlockId(`${pageId}:${String(counter)}`),
      };
    }
    return {
      ...node,
      ...(attributes ? { attrs: attributes } : {}),
      ...(content ? { content } : {}),
    };
  };
  return visit(document_);
}

const inline = (parts: readonly Inline[]): JSONContent[] =>
  parts.map((part) =>
    typeof part === "string" ? { type: "text", text: part } : part,
  );

/** A text run with marks. */
export const marked = (
  text: string,
  ...marks: (string | Mark)[]
): JSONContent => ({
  type: "text",
  text,
  marks: marks.map((m) => (typeof m === "string" ? { type: m } : m)),
});
export const b = (text: string) => marked(text, "bold");
export const em = (text: string) => marked(text, "italic");
export const code = (text: string) => marked(text, "code");
export const hl = (text: string) => marked(text, "highlight");
export const link = (text: string, href: string) =>
  marked(text, { type: "link", attrs: { href, title: "" } });

export const mention = (handle: string): JSONContent => ({
  type: "mention",
  attrs: { handle },
});
export const wiki = (target: string, label = ""): JSONContent => ({
  type: "wikiLink",
  attrs: { target, label },
});
export const issue = (key: string): JSONContent => ({
  type: "issueKey",
  attrs: { key },
});
export const date = (value: string): JSONContent => ({
  type: "date",
  attrs: { date: value },
});
export const math = (literal: string): JSONContent => ({
  type: "mathInline",
  attrs: { literal },
});

export const body = (...content: JSONContent[]): JSONContent => ({
  type: "doc",
  content,
});
export const p = (...parts: Inline[]): JSONContent => ({
  type: "paragraph",
  ...(parts.length > 0 ? { content: inline(parts) } : {}),
});
export const h = (level: number, ...parts: Inline[]): JSONContent => ({
  type: "heading",
  attrs: { level },
  content: inline(parts),
});
export const quote = (...content: JSONContent[]): JSONContent => ({
  type: "blockquote",
  content,
});
export const hr = (): JSONContent => ({ type: "horizontalRule" });

const item = (part: JSONContent | string): JSONContent => ({
  type: "listItem",
  content: [typeof part === "string" ? p(part) : part],
});
export const ul = (...items: (JSONContent | string)[]): JSONContent => ({
  type: "bulletList",
  content: items.map((x) => item(x)),
});
export const ol = (...items: (JSONContent | string)[]): JSONContent => ({
  type: "orderedList",
  attrs: { start: 1 },
  content: items.map((x) => item(x)),
});
export const tasks = (
  ...items: [checked: boolean, ...parts: Inline[]][]
): JSONContent => ({
  type: "taskList",
  content: items.map(([checked, ...parts]) => ({
    type: "taskItem",
    attrs: { checked },
    content: [p(...parts)],
  })),
});

export const callout = (
  kind: string,
  ...content: JSONContent[]
): JSONContent => ({
  type: "callout",
  attrs: { kind },
  content,
});
export const details = (
  summary: string,
  ...content: JSONContent[]
): JSONContent => ({
  type: "details",
  attrs: { summary },
  content,
});
export const columns = (...cols: JSONContent[][]): JSONContent => ({
  type: "columns",
  content: cols.map((content) => ({ type: "column", content })),
});
export const tabs = (
  ...items: [title: string, ...content: JSONContent[]][]
): JSONContent => ({
  type: "tabs",
  content: items.map(([title, ...content]) => ({
    type: "tab",
    attrs: { title },
    content,
  })),
});
export const listBlock = (
  type: string,
  ...items: (JSONContent | string)[]
): JSONContent => ({
  type,
  content: [type === "steps" ? ol(...items) : ul(...items)],
});
export const toc = (): JSONContent => ({ type: "toc" });
export const fence = (
  lang: string,
  body: string,
  arguments_ = "",
): JSONContent => ({
  type: "dataFence",
  attrs: { lang, args: arguments_ },
  content: [{ type: "text", text: body }],
});
export const codeBlock = (language: string, body: string): JSONContent => ({
  type: "codeBlock",
  attrs: { language },
  content: [{ type: "text", text: body }],
});
export const mathBlock = (body: string): JSONContent => ({
  type: "mathBlock",
  content: [{ type: "text", text: body }],
});
export const table = (header: string[], ...rows: string[][]): JSONContent => ({
  type: "table",
  content: [
    {
      type: "tableRow",
      content: header.map((cell) => ({
        type: "tableHeader",
        content: [p(cell)],
      })),
    },
    ...rows.map((row) => ({
      type: "tableRow",
      content: row.map((cell) => ({ type: "tableCell", content: [p(cell)] })),
    })),
  ],
});
export const linkCard = (url: string, title: string): JSONContent => ({
  type: "linkCard",
  attrs: { url, title },
});
export const embed = (url: string): JSONContent => ({
  type: "embed",
  attrs: { url },
});
export const image = (source: string, alt: string): JSONContent => ({
  type: "image",
  attrs: { src: source, alt, title: "", width: "", align: "center" },
});
