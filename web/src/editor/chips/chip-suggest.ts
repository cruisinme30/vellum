// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Extension, type Editor, type Range } from "@tiptap/core";
import { PluginKey } from "@tiptap/pm/state";
import { ReactRenderer } from "@tiptap/react";
import { Suggestion, type SuggestionProps } from "@tiptap/suggestion";

import type { Translate } from "../../i18n/i18n";
import type { SlashItem } from "../slash/items";
import { place } from "../slash/slash-extension";
import {
  SlashMenu,
  type SlashMenuHandle,
  type SlashMenuProperties,
} from "../slash/slash-menu";

import { linkItems, mentionItems, type ChipSources } from "./items";

/** Writes the chosen chip over the typed trigger and query, then a space. */
function insertChip(editor: Editor, range: Range, item: SlashItem): void {
  editor
    .chain()
    .focus()
    .insertContentAt(range, [
      { type: item.node, attrs: item.attrs },
      { type: "text", text: " " },
    ])
    .run();
}

interface MenuOptions {
  readonly t: Translate;
  readonly listId: string;
  readonly labelKey: string;
  readonly emptyKey: string;
}

/** The floating listbox, shared by both menus; focus stays in the text. */
function menuRenderer(
  editor: Editor,
  { t, listId, labelKey, emptyKey }: MenuOptions,
) {
  const setA11y = (open: boolean, active?: string) => {
    const dom = editor.view.dom;
    dom.setAttribute("aria-expanded", String(open));
    if (open) dom.setAttribute("aria-controls", listId);
    else dom.removeAttribute("aria-controls");
    if (active) dom.setAttribute("aria-activedescendant", active);
    else dom.removeAttribute("aria-activedescendant");
  };
  return () => {
    let renderer:
      ReactRenderer<SlashMenuHandle, SlashMenuProperties> | undefined;
    const propertiesFor = (
      properties: SuggestionProps<SlashItem, SlashItem>,
    ): SlashMenuProperties => ({
      items: properties.items,
      command: properties.command,
      t,
      listId,
      labelKey,
      emptyKey,
      onActive: (id) => {
        setA11y(true, id);
      },
    });
    return {
      onStart(properties: SuggestionProps<SlashItem, SlashItem>) {
        renderer = new ReactRenderer(SlashMenu, {
          props: propertiesFor(properties),
          editor: properties.editor,
        });
        document.body.append(renderer.element);
        place(renderer.element, properties.clientRect?.());
        setA11y(true);
      },
      onUpdate(properties: SuggestionProps<SlashItem, SlashItem>) {
        renderer?.updateProps(propertiesFor(properties));
        if (renderer) place(renderer.element, properties.clientRect?.());
      },
      onKeyDown({ event }: { event: KeyboardEvent }) {
        if (event.key === "Escape") return false;
        return renderer?.ref?.onKeyDown(event) ?? false;
      },
      onExit() {
        renderer?.element.remove();
        renderer?.destroy();
        renderer = undefined;
        setA11y(false);
      },
    };
  };
}

/** The query without the `]]` that closes a typed link. */
function withoutClosing(query: string): string {
  let end = query.length;
  while (end > 0 && query[end - 1] === "]") end -= 1;
  return query.slice(0, end);
}

/** Options of {@link ChipSuggest}. */
export interface ChipSuggestOptions {
  readonly t: Translate;
  readonly sources: ChipSources | undefined;
  /** The current time, for date entries; injectable for tests. */
  readonly now: () => Date;
}

/**
 * Autocomplete for inline chips (#93): `@` offers people and dates, `[[`
 * offers pages and, with a server, issues. Typing the full token still
 * works through each chip's input rule.
 */
export const ChipSuggest = Extension.create<ChipSuggestOptions>({
  name: "chipSuggest",

  addOptions() {
    return { t: (id: string) => id, sources: undefined, now: () => new Date() };
  },

  addProseMirrorPlugins() {
    const { t, sources, now } = this.options;
    if (!sources) return [];
    const editor = this.editor;
    const command = ({
      editor: target,
      range,
      props,
    }: {
      editor: Editor;
      range: Range;
      props: SlashItem;
    }) => {
      insertChip(target, range, props);
    };
    return [
      Suggestion<SlashItem, SlashItem>({
        editor,
        pluginKey: new PluginKey("vellumMentionSuggest"),
        char: "@",
        allowSpaces: false,
        items: ({ query }) => mentionItems(query, sources, t, now()),
        command,
        render: menuRenderer(editor, {
          t,
          listId: "vl-mention-list",
          labelKey: "chips-mention-label",
          emptyKey: "chips-mention-empty",
        }),
      }),
      Suggestion<SlashItem, SlashItem>({
        editor,
        pluginKey: new PluginKey("vellumLinkSuggest"),
        char: "[[",
        allowSpaces: true,
        items: ({ query }) => linkItems(withoutClosing(query), sources, t),
        command,
        render: menuRenderer(editor, {
          t,
          listId: "vl-link-list",
          labelKey: "chips-link-label",
          emptyKey: "chips-link-empty",
        }),
      }),
    ];
  },
});
