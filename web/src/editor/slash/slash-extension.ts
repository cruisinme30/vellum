// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Extension } from "@tiptap/core";
import { PluginKey } from "@tiptap/pm/state";
import { ReactRenderer } from "@tiptap/react";
import { Suggestion, type SuggestionProps } from "@tiptap/suggestion";

import type { Translate } from "../../i18n/i18n";

import { insertSlashItem } from "./insert";
import { filterSlashItems, type SlashItem } from "./items";
import {
  SlashMenu,
  type SlashMenuHandle,
  type SlashMenuProperties as SlashMenuProperties,
} from "./slash-menu";

/** Plugin key of the slash menu. */
export const slashKey = new PluginKey("vellumSlash");

const LIST_ID = "vl-slash-list";

function place(element: HTMLElement, rect: DOMRect | null | undefined) {
  if (!rect) {
    return;
  }
  const below = rect.bottom + 6;
  const room = globalThis.innerHeight - below;
  element.style.position = "fixed";
  element.style.left = `${String(Math.max(8, Math.min(rect.left, globalThis.innerWidth - 328)))}px`;
  element.style.top = room < 240 ? "" : `${String(below)}px`;
  element.style.bottom = room < 240 ? `${String(globalThis.innerHeight - rect.top + 6)}px` : "";
  element.style.zIndex = "40";
}

/**
 * The `/` menu (R-ED): lists every block from the schema, filters as the
 * user types and inserts on Enter. Focus never leaves the text; the editor
 * points `aria-activedescendant` at the highlighted option.
 */
export const SlashCommand = Extension.create<{ t: Translate }>({
  name: "slashCommand",

  addOptions() {
    return { t: (id: string) => id };
  },

  addProseMirrorPlugins() {
    const { t } = this.options;
    const editor = this.editor;
    const setA11y = (open: boolean, active?: string) => {
      const dom = editor.view.dom;
      dom.setAttribute("aria-expanded", String(open));
      if (open) {
        dom.setAttribute("aria-controls", LIST_ID);
      } else {
        dom.removeAttribute("aria-controls");
      }
      if (active) {
        dom.setAttribute("aria-activedescendant", active);
      } else {
        dom.removeAttribute("aria-activedescendant");
      }
    };
    return [
      Suggestion<SlashItem, SlashItem>({
        editor,
        pluginKey: slashKey,
        char: "/",
        allowSpaces: false,
        startOfLine: false,
        items: ({ query }) => filterSlashItems(query),
        command: ({ editor: target, range, props }) => {
          insertSlashItem(target, range, props);
        },
        render: () => {
          let renderer: ReactRenderer<SlashMenuHandle, SlashMenuProperties> | undefined;
          const propertiesFor = (
            properties: SuggestionProps<SlashItem, SlashItem>,
          ): SlashMenuProperties => ({
            items: properties.items,
            command: properties.command,
            t,
            listId: LIST_ID,
            onActive: (id) => {
              setA11y(true, id);
            },
          });
          return {
            onStart(properties) {
              renderer = new ReactRenderer(SlashMenu, {
                props: propertiesFor(properties),
                editor: properties.editor,
              });
              document.body.append(renderer.element);
              place(renderer.element, properties.clientRect?.());
              setA11y(true);
            },
            onUpdate(properties) {
              renderer?.updateProps(propertiesFor(properties));
              if (renderer) {
                place(renderer.element, properties.clientRect?.());
              }
            },
            onKeyDown({ event }) {
              if (event.key === "Escape") {
                return false;
              }
              return renderer?.ref?.onKeyDown(event) ?? false;
            },
            onExit() {
              renderer?.element.remove();
              renderer?.destroy();
              renderer = undefined;
              setA11y(false);
            },
          };
        },
      }),
    ];
  },
});
