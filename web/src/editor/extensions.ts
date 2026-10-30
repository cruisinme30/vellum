// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { AnyExtension } from "@tiptap/core";
import { Highlight } from "@tiptap/extension-highlight";
import {
  BulletList,
  OrderedList,
  TaskItem,
  TaskList,
} from "@tiptap/extension-list";
import { Subscript } from "@tiptap/extension-subscript";
import { Superscript } from "@tiptap/extension-superscript";
import { Placeholder } from "@tiptap/extensions";
import { StarterKit } from "@tiptap/starter-kit";

import type { Translate } from "../i18n/i18n";

import { BlockIds } from "./block-id-extension";
import { ChipSuggest } from "./chips/chip-suggest";
import { issueChipView } from "./chips/issue-chip";
import type { ChipSources } from "./chips/items";
import {
  CodeBlock,
  DataFence,
  Details,
  Embed,
  Image,
  LinkCard,
  MathBlock,
  Tab,
  Table,
  TableCell,
  TableHeader,
  TableRow,
  Tabs,
  Toc,
} from "./nodes/blocks";
import {
  Callout,
  Cards,
  Column,
  Columns,
  Container,
  Facts,
  Pages,
  Progress,
  Stats,
  Steps,
} from "./nodes/generic";
import {
  DateChip,
  InlineImage,
  IssueKey,
  MathInline,
  Mention,
  WikiLink,
  type ResolvePage,
} from "./nodes/inline";
import { nodeSpec } from "./schema/spec";
import { SchemaAttributes } from "./schema-attributes";
import { SlashCommand } from "./slash/slash-extension";

/** What the editor needs from its host. */
export interface ExtensionOptions {
  readonly t: Translate;
  readonly resolvePage: ResolvePage;
  /** Off when Yjs owns undo history (always, outside tests). */
  readonly history: boolean;
  /**
   * People, pages and issues for the `@` and `[[` menus and live issue chips.
   */
  readonly chips?: ChipSources | undefined;
}

/**
 * Every extension of the Vellum editor: one per node and mark of
 * schema/blocks.json (checked by extensions.test.ts), plus block IDs, the
 * slash menu and placeholders. Collaboration is added by the host.
 */
export function vellumExtensions({
  t,
  resolvePage,
  history,
  chips,
}: ExtensionOptions): AnyExtension[] {
  return [
    StarterKit.configure({
      codeBlock: false,
      bulletList: false,
      orderedList: false,
      underline: false,
      trailingNode: false,
      undoRedo: history ? {} : false,
      link: {
        openOnClick: false,
        autolink: true,
        // http, https and mailto are linkify built-ins: registering them again
        // as custom protocols only warns ("already initialized") from the
        // second editor on.
        isAllowedUri: (url, context) =>
          /^(?:https?:|mailto:|\/|#)/i.test(url) &&
          context.defaultValidate(url),
        // Tiptap removes an attribute set to null: links never open a new
        // window by default.
        // eslint-disable-next-line unicorn/no-null
        HTMLAttributes: { rel: "noopener noreferrer", target: null },
      },
    }),
    // Lists may mix in tasks, so ordered and mixed task lists fit.
    BulletList.extend({ content: nodeSpec("bulletList").content }),
    OrderedList.extend({ content: nodeSpec("orderedList").content }),
    CodeBlock,
    MathBlock,
    TaskList,
    // Keeps the item's `{...}` attribute span (D82) through edits.
    TaskItem.extend({
      addAttributes() {
        return {
          ...this.parent?.(),
          attributes: {
            default: "",
            parseHTML: (element) => element.dataset["attributes"] ?? "",
            renderHTML: (attributes) => {
              const span = String(attributes["attributes"] ?? "");
              return span === "" ? {} : { "data-attributes": span };
            },
          },
        };
      },
    }).configure({ nested: true }),
    Highlight,
    // `~sub~` and `^super^` cannot nest in Vellum Markdown (schema `excludes`).
    Subscript.extend({ excludes: "superscript" }),
    Superscript.extend({ excludes: "subscript" }),
    Table,
    TableRow,
    TableHeader,
    TableCell,
    Callout,
    Details,
    Columns,
    Column,
    Tabs,
    Tab,
    Toc,
    Stats,
    Facts,
    Progress,
    Cards,
    Steps,
    Pages,
    Container,
    LinkCard,
    Embed,
    Image,
    DataFence,
    Mention,
    WikiLink.configure({ resolve: resolvePage }),
    chips?.issueStatus === undefined && chips?.openIssue === undefined
      ? IssueKey
      : IssueKey.extend({ addNodeView: () => issueChipView(chips, t) }),
    DateChip,
    MathInline,
    InlineImage,
    BlockIds,
    SchemaAttributes,
    SlashCommand.configure({ t }),
    ChipSuggest.configure({ t, sources: chips }),
    Placeholder.configure({
      placeholder: ({ node }) =>
        node.type.name === "paragraph" ? t("editor-placeholder") : "",
    }),
  ];
}
