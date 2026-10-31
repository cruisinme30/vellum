// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { mergeAttributes, Node, textblockTypeInputRule } from "@tiptap/core";
import { CodeBlock as CodeBlockBase } from "@tiptap/extension-code-block";
import {
  Table as TableBase,
  TableCell as CellBase,
  TableHeader as HeaderBase,
} from "@tiptap/extension-table";
import { ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react";
import type { ComponentType } from "react";

import type { NodeName } from "../schema/blocks.gen";
import { nodeSpec, tiptapAttributes } from "../schema/spec";

import { DataFenceView } from "./data-fence-view";
import { DetailsView } from "./details-view";
import { EmbedView, ImageView, LinkCardView } from "./media-views";
import { TabsView, TabView } from "./tabs-view";
import { TocView } from "./toc-view";

const fromSchema = (name: NodeName) => {
  const spec = nodeSpec(name);
  return {
    name: spec.name,
    ...(spec.group === "" ? {} : { group: spec.group }),
    ...(spec.content === "" ? {} : { content: spec.content }),
    ...(spec.marks === undefined ? {} : { marks: spec.marks }),
    ...(spec.code === true ? { code: true } : {}),
    ...(spec.atom === true ? { atom: true } : {}),
    ...(spec.defining === true ? { defining: true } : {}),
    ...(spec.isolating === true ? { isolating: true } : {}),
    ...(spec.draggable === true ? { draggable: true } : {}),
    addAttributes: () => tiptapAttributes(spec),
    parseHTML: () => [{ tag: `div[data-type="${spec.name}"]` }],
  };
};

/** A fenced code block; the language is free text, as in the info string. */
export const CodeBlock = CodeBlockBase.extend({
  addAttributes() {
    return {
      // `args` and `attributes`: the rest of the info string, kept verbatim.
      ...tiptapAttributes(nodeSpec("codeBlock")),
      language: {
        default: "",
        parseHTML: (element) =>
          /language-(\S+)/.exec(
            element.querySelector("code")?.className ?? "",
          )?.[1] ?? "",
        renderHTML: (attributes) => {
          const language =
            typeof attributes["language"] === "string"
              ? attributes["language"]
              : "";
          return language === "" ? {} : { "data-language": language };
        },
      },
    };
  },
}).configure({
  HTMLAttributes: { class: "vl-code" },
  enableTabIndentation: true,
});

/** `$$` display math. */
export const MathBlock = Node.create({
  ...fromSchema("mathBlock"),
  // Above the default 50, or the code block's `pre` rule takes it.
  parseHTML: () => [
    {
      tag: 'pre[data-type="mathBlock"]',
      preserveWhitespace: "full",
      priority: 60,
    },
  ],
  renderHTML: ({ HTMLAttributes }) => [
    "pre",
    mergeAttributes(HTMLAttributes, {
      "data-type": "mathBlock",
      class: "vl-math",
    }),
    ["code", 0],
  ],
  addInputRules() {
    return [textblockTypeInputRule({ find: /^\$\$\s$/, type: this.type })];
  },
});

const cellContent = { content: nodeSpec("tableCell").content };
const alignAttribute = () => tiptapAttributes(nodeSpec("tableCell"));

/** GFM tables: cells hold one paragraph, columns carry an alignment. */
export const Table = TableBase.configure({
  resizable: false,
  HTMLAttributes: { class: "vl-table" },
});

export const TableHeader = HeaderBase.extend({
  ...cellContent,
  addAttributes() {
    return { ...this.parent?.(), ...alignAttribute() };
  },
});
export const TableCell = CellBase.extend({
  ...cellContent,
  addAttributes() {
    return { ...this.parent?.(), ...alignAttribute() };
  },
});

/** `::: details`: a collapsible section. */
export const Details = Node.create({
  ...fromSchema("details"),
  renderHTML: ({ HTMLAttributes }) => [
    "div",
    mergeAttributes(HTMLAttributes, {
      "data-type": "details",
      class: "vl-block vl-details",
    }),
    0,
  ],
  addNodeView: () => ReactNodeViewRenderer(DetailsView),
});

/** `::: tabs`. */
export const Tabs = Node.create({
  ...fromSchema("tabs"),
  renderHTML: ({ HTMLAttributes }) => [
    "div",
    mergeAttributes(HTMLAttributes, {
      "data-type": "tabs",
      class: "vl-block vl-tabs",
    }),
    0,
  ],
  addNodeView: () => ReactNodeViewRenderer(TabsView),
});

/** One tab of `::: tabs`. */
export const Tab = Node.create({
  ...fromSchema("tab"),
  renderHTML: ({ HTMLAttributes }) => [
    "div",
    mergeAttributes(HTMLAttributes, { "data-type": "tab", class: "vl-tab" }),
    0,
  ],
  addNodeView: () => ReactNodeViewRenderer(TabView),
});

/** `::: toc`. */
export const Toc = Node.create({
  ...fromSchema("toc"),
  parseHTML: () => [{ tag: 'nav[data-type="toc"]' }],
  renderHTML: ({ HTMLAttributes }) => [
    "nav",
    mergeAttributes(HTMLAttributes, {
      "data-type": "toc",
      class: "vl-block vl-toc",
    }),
  ],
  addNodeView: () => ReactNodeViewRenderer(TocView),
});

const atomBlock = (name: NodeName, view: ComponentType<ReactNodeViewProps>) =>
  Node.create({
    ...fromSchema(name),
    renderHTML: ({ HTMLAttributes }) => [
      "div",
      mergeAttributes(HTMLAttributes, {
        "data-type": name,
        class: `vl-block vl-${name}`,
      }),
    ],
    addNodeView: () => ReactNodeViewRenderer(view),
  });

/** `::: link <url>`. */
export const LinkCard = atomBlock("linkCard", LinkCardView);
/** `::: embed <url>`, sandboxed. */
export const Embed = atomBlock("embed", EmbedView);
/** A block image. */
export const Image = atomBlock("image", ImageView);

/** A data fence: chart, plan, diagram, calculator or collection view. */
export const DataFence = Node.create({
  ...fromSchema("dataFence"),
  parseHTML: () => [
    { tag: 'div[data-type="dataFence"]', contentElement: "code" },
  ],
  renderHTML: ({ HTMLAttributes }) => [
    "div",
    mergeAttributes(HTMLAttributes, {
      "data-type": "dataFence",
      class: "vl-block vl-fence",
    }),
    ["pre", ["code", 0]],
  ],
  addNodeView: () => ReactNodeViewRenderer(DataFenceView),
});

export { TableRow } from "@tiptap/extension-table";
