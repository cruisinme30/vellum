// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Extension, type Editor } from "@tiptap/core";
import type { Node as PmNode } from "@tiptap/pm/model";
import { Plugin, PluginKey, type EditorState } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

import { isBlockId, newBlockId } from "../block-id";

/** Where a new comment thread or issue points: a block and the selected text. */
export interface Anchor {
  /** The innermost block around the selection, when it has an ID yet. */
  readonly blockId: string | undefined;
  /** The selected text, trimmed to {@link QUOTE_LIMIT} characters; empty for a caret. */
  readonly quote: string;
}

/** The longest quote kept on a thread or issue. */
const QUOTE_LIMIT = 500;

/** The block and text the selection is on. */
export function anchorOf(state: EditorState): Anchor {
  const { $from, from, to } = state.selection;
  let blockId: string | undefined;
  for (let depth = $from.depth; depth > 0; depth -= 1) {
    const id: unknown = $from.node(depth).attrs["blockId"];
    if (isBlockId(id)) {
      blockId = id;
      break;
    }
  }
  if (blockId === undefined) {
    // A selected block node (an image, a fence) carries the ID itself.
    const id: unknown = state.doc.nodeAt(from)?.attrs["blockId"];
    if (isBlockId(id)) blockId = id;
  }
  const quote = state.doc.textBetween(from, to, " ").trim().slice(0, QUOTE_LIMIT);
  return { blockId, quote };
}

/**
 * The anchor for the selection, first giving its block an ID when it has
 * none yet and the page is editable, so a thread or issue can point at it.
 */
export function ensureAnchor(editor: Editor): Anchor {
  const anchor = anchorOf(editor.state);
  if (anchor.blockId !== undefined || !editor.isEditable) return anchor;
  const { $from } = editor.state.selection;
  for (let depth = $from.depth; depth > 0; depth -= 1) {
    const node = $from.node(depth);
    if ("blockId" in node.attrs) {
      const blockId = newBlockId();
      editor.view.dispatch(
        editor.state.tr.setNodeAttribute($from.before(depth), "blockId", blockId),
      );
      return { ...anchor, blockId };
    }
  }
  return anchor;
}

/** The comment target for a page or one of its blocks (`<doc id>#blk-…`). */
export function targetOf(documentId: string, blockId?: string): string {
  return blockId === undefined ? documentId : `${documentId}#${blockId}`;
}

/** The position of the block with this ID, if the document has it. */
export function blockPosition(document: PmNode, blockId: string): number | undefined {
  let found: number | undefined;
  document.descendants((node, pos) => {
    if (found !== undefined) return false;
    if (node.attrs["blockId"] === blockId) {
      found = pos;
      return false;
    }
    return true;
  });
  return found;
}

const markersKey = new PluginKey<DecorationSet>("vellumCommentMarkers");

function decorate(document: PmNode, blocks: ReadonlySet<string>, active?: string): DecorationSet {
  if (blocks.size === 0) return DecorationSet.empty;
  const decorations: Decoration[] = [];
  document.descendants((node, pos) => {
    const id: unknown = node.attrs["blockId"];
    if (typeof id === "string" && blocks.has(id)) {
      decorations.push(
        Decoration.node(pos, pos + node.nodeSize, {
          class: id === active ? "vl-commented vl-commented-active" : "vl-commented",
        }),
      );
    }
    return true;
  });
  return DecorationSet.create(document, decorations);
}

interface MarkerMeta {
  readonly blocks: ReadonlySet<string>;
  readonly active: string | undefined;
}

/**
 * Highlights blocks with open comment threads (#94). The comments panel
 * says which blocks those are with {@link setCommentMarkers}; the markers
 * follow the blocks as the document changes.
 */
export const CommentMarkers = Extension.create({
  name: "commentMarkers",

  addProseMirrorPlugins() {
    let current: MarkerMeta = { blocks: new Set(), active: undefined };
    return [
      new Plugin<DecorationSet>({
        key: markersKey,
        state: {
          init: () => DecorationSet.empty,
          apply(tr, old) {
            const meta = tr.getMeta(markersKey) as MarkerMeta | undefined;
            if (meta) {
              current = meta;
              return decorate(tr.doc, meta.blocks, meta.active);
            }
            return tr.docChanged ? decorate(tr.doc, current.blocks, current.active) : old;
          },
        },
        props: {
          decorations: (state) => markersKey.getState(state),
        },
      }),
    ];
  },
});

/** Marks these blocks as commented, and `active` as the thread being read. */
export function setCommentMarkers(
  editor: Editor,
  blocks: ReadonlySet<string>,
  active?: string,
): void {
  if (editor.isDestroyed) return;
  const meta: MarkerMeta = { blocks, active };
  editor.view.dispatch(editor.state.tr.setMeta(markersKey, meta).setMeta("addToHistory", false));
}
