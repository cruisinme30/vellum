// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

interface MoveMenuProperties {
  /** The card's title, for the button's name. */
  readonly title: string;
  /** Every column, by label. */
  readonly targets: readonly string[];
  /** The column the card is in. */
  readonly current: number;
  readonly onMove: (to: number) => void;
}

/** A card's "Move to…" menu: the keyboard way to change its column. */
export function MoveMenu({ title, targets, current, onMove }: MoveMenuProperties) {
  const t = useT();
  const id = useId();
  const [open, setOpen] = useState(false);
  const menu = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const choices = targets
    .map((label, index) => ({ label, index }))
    .filter((c) => c.index !== current);

  useEffect(() => {
    if (open) {
      menu.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    }
  }, [open]);

  const close = () => {
    setOpen(false);
    button.current?.focus();
  };
  const pick = (to: number) => {
    setOpen(false);
    onMove(to);
  };
  const onKeyDown = (event: KeyboardEvent) => {
    event.stopPropagation();
    const items = [...(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
    const at = items.indexOf(document.activeElement as HTMLElement);
    const target = { ArrowDown: at + 1, ArrowUp: at - 1, Home: 0, End: items.length - 1 }[
      event.key
    ];
    if (target !== undefined) {
      event.preventDefault();
      items[(target + items.length) % items.length]?.focus();
    } else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      close();
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const to = Number((document.activeElement as HTMLElement | null)?.dataset["to"] ?? -1);
      if (to >= 0) {
        pick(to);
      }
    }
  };

  return (
    <span className="vl-menu-anchor vl-card-menu">
      <button
        ref={button}
        type="button"
        className="vl-icon-button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-label={t("collection-move-label", { title })}
        onClick={() => {
          setOpen(!open);
        }}
      >
        <Icon name="drive_file_move" size={20} />
      </button>
      {open && (
        <div
          className="vl-menu"
          role="menu"
          tabIndex={-1}
          id={id}
          ref={menu}
          onKeyDown={onKeyDown}
          onBlur={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget)) {
              setOpen(false);
            }
          }}
          onClick={(event) => {
            const to = (event.target as HTMLElement).closest<HTMLElement>("[data-to]")?.dataset[
              "to"
            ];
            if (to !== undefined) {
              pick(Number(to));
            }
          }}
        >
          {choices.map(({ label, index }) => (
            <div key={label} role="menuitem" tabIndex={-1} data-to={index}>
              {t("collection-move-to", { group: label })}
            </div>
          ))}
        </div>
      )}
    </span>
  );
}
