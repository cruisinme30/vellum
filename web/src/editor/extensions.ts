// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { AnyExtension } from "@tiptap/core";
import { Highlight } from "@tiptap/extension-highlight";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { Subscript } from "@tiptap/extension-subscript";
import { Superscript } from "@tiptap/extension-superscript";
import { Placeholder } from "@tiptap/extensions";
import { StarterKit } from "@tiptap/starter-kit";

import type { Translate } from "../i18n/i18n";

import { BlockIds } from "./block-id-extension";
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
  Facts,
  Pages,
  Progress,
  Stats,
  Steps,
} from "./nodes/generic";
import {
  DateChip,
  IssueKey,
  MathInline,
  Mention,
  WikiLink,
  type ResolvePage,
} from "./nodes/inline";
import { SlashCommand } from "./slash/slash-extension";

/** What the editor needs from its host. */
export interface ExtensionOptions {
  readonly t: Translate;
  readonly resolvePage: ResolvePage;
  /** Off when Yjs owns undo history (always, outside tests). */
  readonly history: boolean;
}

/**
 * Every extension of the Vellum editor: one per node and mark of
 * schema/blocks.json (checked by extensions.test.ts), plus block IDs, the
 * slash menu and placeholders. Collaboration is added by the host.
 */
export function vellumExtensions({ t, resolvePage, history }: ExtensionOptions): AnyExtension[] {
  return [
    StarterKit.configure({
      codeBlock: false,
      underline: false,
      trailingNode: false,
      undoRedo: history ? {} : false,
      link: {
        openOnClick: false,
        autolink: true,
        protocols: ["http", "https", "mailto"],
        isAllowedUri: (url, context) =>
          /^(?:https?:|mailto:|\/|#)/i.test(url) && context.defaultValidate(url),
        // Tiptap removes an attribute set to null: links never open a new window by default.
        // eslint-disable-next-line unicorn/no-null
        HTMLAttributes: { rel: "noopener noreferrer", target: null },
      },
    }),
    CodeBlock,
    MathBlock,
    TaskList,
    TaskItem.configure({ nested: true }),
    Highlight,
    Subscript,
    Superscript,
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
    LinkCard,
    Embed,
    Image,
    DataFence,
    Mention,
    WikiLink.configure({ resolve: resolvePage }),
    IssueKey,
    DateChip,
    MathInline,
    BlockIds,
    SlashCommand.configure({ t }),
    Placeholder.configure({
      placeholder: ({ node }) => (node.type.name === "paragraph" ? t("editor-placeholder") : ""),
    }),
  ];
}
