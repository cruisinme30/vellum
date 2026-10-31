// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type Ref,
} from "react";

import type { Translate } from "../../i18n/i18n";
import { Icon } from "../../shell/icon";

import type { SlashItem } from "./items";

/** What the suggestion plugin hands the menu. */
export interface SlashMenuProperties {
  readonly items: readonly SlashItem[];
  readonly command: (item: SlashItem) => void;
  readonly t: Translate;
  /** The DOM id prefix; options are `${listId}-${item.id}`. */
  readonly listId: string;
  readonly onActive: (optionId: string | undefined) => void;
  readonly ref?: Ref<SlashMenuHandle>;
  /**
   * Message IDs of the list's name and its empty state; the block menu's by
   * default.
   */
  readonly labelKey?: string;
  readonly emptyKey?: string;
}

/**
 * Lets the editor forward arrow keys and Enter while focus stays in the text.
 */
export interface SlashMenuHandle {
  onKeyDown: (event: KeyboardEvent) => boolean;
}

/** The slash menu: a listbox the editor drives with `aria-activedescendant`. */
export function SlashMenu({
  items,
  command,
  t,
  listId,
  onActive,
  ref,
  labelKey = "slash-label",
  emptyKey = "slash-empty",
}: SlashMenuProperties) {
  const [active, setActive] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const index = Math.min(active, Math.max(0, items.length - 1));
  const current = items[index];

  // A new query starts at the first match (state adjusted during render).
  const [shown, setShown] = useState(items);
  if (shown !== items) {
    setShown(items);
    setActive(0);
  }
  useEffect(() => {
    onActive(current ? `${listId}-${current.id}` : undefined);
    if (current) {
      const option = list.current?.querySelector(
        "#" + CSS.escape(`${listId}-${current.id}`),
      );
      if (option && "scrollIntoView" in option) {
        option.scrollIntoView({ block: "nearest" });
      }
    }
  }, [current, listId, onActive]);

  useImperativeHandle(ref, () => ({
    onKeyDown(event) {
      if (items.length === 0) {
        return false;
      }
      switch (event.key) {
        case "ArrowDown": {
          setActive((index + 1) % items.length);
          return true;
        }
        case "ArrowUp": {
          setActive((index - 1 + items.length) % items.length);
          return true;
        }
        case "Enter":
        case "Tab": {
          if (current) {
            command(current);
          }
          return true;
        }
        default: {
          return false;
        }
      }
    },
  }));

  if (items.length === 0) {
    return (
      <div className="vl-slash" role="status">
        <p className="vl-muted">{t(emptyKey)}</p>
      </div>
    );
  }

  // Consecutive entries of one section form a group, so the visual order
  // is always the order the arrow keys walk.
  const runs: {
    section: string;
    entries: { item: SlashItem; position: number }[];
  }[] = [];
  for (const [position, item] of items.entries()) {
    const last = runs.at(-1);
    if (last?.section === item.section) {
      last.entries.push({ item, position });
    } else {
      runs.push({ section: item.section, entries: [{ item, position }] });
    }
  }
  return (
    <div
      className="vl-slash"
      id={listId}
      role="listbox"
      aria-label={t(labelKey)}
      ref={list}
    >
      {runs.map((run, runIndex) => (
        <div
          key={`${run.section}-${String(runIndex)}`}
          role="group"
          aria-labelledby={`${listId}-group-${String(runIndex)}`}
        >
          <div
            className="vl-slash-section"
            id={`${listId}-group-${String(runIndex)}`}
          >
            {run.section}
          </div>
          {run.entries.map(({ item, position }) => (
            <div
              key={item.id}
              id={`${listId}-${item.id}`}
              role="option"
              tabIndex={-1}
              aria-selected={position === index}
              className="vl-slash-item"
              onMouseDown={(event) => {
                event.preventDefault();
                command(item);
              }}
              onMouseEnter={() => {
                setActive(position);
              }}
            >
              <Icon name={item.icon} />
              <span>
                <strong>{item.title}</strong>
                <small>{item.description}</small>
              </span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
