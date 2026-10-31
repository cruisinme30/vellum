// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
} from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import type { Choice } from "./editing";

interface OptionPickerProperties {
  readonly label: string;
  readonly choices: readonly Choice[];
  readonly selected: readonly string[];
  readonly multiple: boolean;
  /** The picks, in choice order. */
  readonly onCommit: (ids: string[]) => void;
  readonly onCancel: () => void;
}

const MOVES: Readonly<Record<string, (at: number, last: number) => number>> = {
  ArrowDown: (at, last) => Math.min(at + 1, last),
  ArrowUp: (at) => Math.max(at - 1, 0),
  Home: () => 0,
  End: (_, last) => last,
};

/**
 * A listbox popover for select, multi-select, person and relation values.
 * Arrows move, Enter picks (or, for several, keeps the picks), Space
 * toggles one of several, Escape cancels. Leaving it keeps the picks of a
 * multi-select and cancels a single pick.
 */
export function OptionPicker({
  label,
  choices,
  selected,
  multiple,
  onCommit: commit,
  onCancel: cancel,
}: OptionPickerProperties) {
  const t = useT();
  // Enter and a click close the list; the blur that may follow must not act
  // again.
  const done = useRef(false);
  const onCommit = (ids: string[]) => {
    done.current = true;
    commit(ids);
  };
  const onCancel = () => {
    done.current = true;
    cancel();
  };
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  // A single pick can be cleared with the "None" entry at the top.
  const entries: readonly Choice[] = multiple
    ? choices
    : [{ id: "", label: t("collection-none") }, ...choices];
  const [picked, setPicked] = useState<readonly string[]>(selected);
  const [active, setActive] = useState(() =>
    Math.max(
      0,
      entries.findIndex((entry) => entry.id === (selected[0] ?? "")),
    ),
  );
  useEffect(() => {
    box.current?.focus();
    // The list may open near the bottom of a scrolling table.
    box.current?.scrollIntoView({ block: "nearest" });
  }, []);

  const ordered = (ids: readonly string[]) =>
    choices.filter((c) => ids.includes(c.id)).map((c) => c.id);
  const choose = (entry: Choice | undefined) => {
    if (!entry) {
      return;
    }
    if (!multiple) {
      onCommit(entry.id === "" ? [] : [entry.id]);
      return;
    }
    setPicked(
      picked.includes(entry.id)
        ? picked.filter((p) => p !== entry.id)
        : [...picked, entry.id],
    );
  };

  const onKeyDown = (event: KeyboardEvent) => {
    const move = MOVES[event.key];
    event.stopPropagation();
    if (move) {
      event.preventDefault();
      setActive(move(active, entries.length - 1));
      return;
    }
    switch (event.key) {
      case "Escape": {
        event.preventDefault();
        onCancel();
        break;
      }
      case "Enter": {
        event.preventDefault();
        if (multiple) {
          onCommit(ordered(picked));
        } else {
          choose(entries[active]);
        }
        break;
      }
      case " ": {
        event.preventDefault();
        choose(entries[active]);
        break;
      }
      default:
    }
  };
  const onClick = (event: MouseEvent) => {
    const option = (event.target as HTMLElement).closest<HTMLElement>(
      "[role=option]",
    );
    const index = Number(option?.dataset["index"] ?? -1);
    setActive(Math.max(index, 0));
    choose(entries[index]);
  };

  return (
    <div
      ref={box}
      className="vl-picker"
      role="listbox"
      aria-label={label}
      aria-multiselectable={multiple || undefined}
      aria-activedescendant={
        entries.length > 0 ? `${id}-${String(active)}` : undefined
      }
      tabIndex={0}
      onKeyDown={onKeyDown}
      onClick={onClick}
      onMouseDown={(event) => {
        // Keep focus in the list while clicking an option.
        event.preventDefault();
      }}
      onBlur={(event) => {
        if (
          !done.current &&
          !event.currentTarget.contains(event.relatedTarget)
        ) {
          if (multiple) {
            onCommit(ordered(picked));
          } else {
            onCancel();
          }
        }
      }}
    >
      {entries.length === 0 && (
        <div className="vl-picker-empty">{t("collection-no-choices")}</div>
      )}
      {entries.map((entry, index) => {
        const isPicked = multiple
          ? picked.includes(entry.id)
          : (selected[0] ?? "") === entry.id;
        return (
          <div
            key={entry.id}
            id={`${id}-${String(index)}`}
            role="option"
            aria-selected={isPicked}
            data-index={index}
            data-active={index === active || undefined}
            className="vl-picker-option"
          >
            <span className="vl-picker-check">
              {isPicked && <Icon name="check" size={18} />}
            </span>
            <span className="vl-chip" data-color={entry.color}>
              {entry.label}
            </span>
          </div>
        );
      })}
    </div>
  );
}
