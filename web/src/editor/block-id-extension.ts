// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Extension } from "@tiptap/core";
import { Plugin, PluginKey, type Transaction } from "@tiptap/pm/state";
import { ySyncPluginKey } from "@tiptap/y-tiptap";

import { isBlockId, newBlockId } from "./block-id";
import { BLOCK_ID_TYPES } from "./schema/spec";

/** Plugin key of the block-ID plugin. */
const blockIdKey = new PluginKey("vellumBlockIds");

const isRemote = (tr: Transaction) => {
  const meta = tr.getMeta(ySyncPluginKey) as { isChangeOrigin?: boolean } | undefined;
  return meta?.isChangeOrigin === true;
};

/**
 * Gives every block a stable `blockId` (`blk-xxxxxxxx`) so comments, tasks,
 * issue back-links and AI citations can point at it. IDs are assigned on
 * local edits only; changes from peers already carry theirs. A split or a
 * paste that duplicates an ID gets a fresh one for the copy.
 */
export const BlockIds = Extension.create<{ generate: () => string }>({
  name: "blockIds",

  addOptions() {
    return { generate: newBlockId };
  },

  addGlobalAttributes() {
    return [
      {
        types: [...BLOCK_ID_TYPES],
        attributes: {
          blockId: {
            // ProseMirror attributes use null for "unset".
            // eslint-disable-next-line unicorn/no-null
            default: null,
            keepOnSplit: false,
            // eslint-disable-next-line unicorn/no-null
            parseHTML: (element) => element.dataset["blockId"] ?? null,
            renderHTML: (attributes) =>
              typeof attributes["blockId"] === "string"
                ? { "data-block-id": attributes["blockId"] }
                : {},
          },
        },
      },
    ];
  },

  addProseMirrorPlugins() {
    const types = new Set(BLOCK_ID_TYPES);
    const { generate } = this.options;
    return [
      new Plugin({
        key: blockIdKey,
        appendTransaction(transactions, _old, state) {
          const local = transactions.some((tr) => tr.docChanged && !isRemote(tr));
          if (!local) {
            // ProseMirror's "no transaction to append".
            // eslint-disable-next-line unicorn/no-null
            return null;
          }
          const seen = new Set<string>();
          const tr = state.tr;
          state.doc.descendants((node, pos) => {
            if (!types.has(node.type.name)) {
              return;
            }
            const id: unknown = node.attrs["blockId"];
            if (isBlockId(id) && !seen.has(id)) {
              seen.add(id);
              return;
            }
            let fresh = generate();
            while (seen.has(fresh)) {
              fresh = generate();
            }
            seen.add(fresh);
            tr.setNodeAttribute(pos, "blockId", fresh);
          });
          // eslint-disable-next-line unicorn/no-null
          return tr.docChanged ? tr.setMeta("addToHistory", false) : null;
        },
      }),
    ];
  },
});
