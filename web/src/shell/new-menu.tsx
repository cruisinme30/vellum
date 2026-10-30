// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { useT } from "../i18n/i18n";

/** What the New button can make. */
export interface NewActions {
  readonly page: () => void;
  /** Absent without a server, which is where books live. */
  readonly book?: (() => void) | undefined;
}

/**
 * The New button (D323). With only pages to make it makes one at once, as
 * before; once books can be made too it opens a menu of what to make
 * (WAI-ARIA menu button), so a book is one choice away.
 */
export function NewButton({
  actions,
  className,
  children,
}: {
  readonly actions: NewActions;
  readonly className: string;
  readonly children: ReactNode;
}) {
  const t = useT();
  const id = useId();
  const [open, setOpen] = useState(false);
  const menu = useRef<HTMLDivElement>(null);
  const { book } = actions;

  useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    const onPointer = (event: PointerEvent) => {
      if (!menu.current?.parentElement?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointer);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
    };
  }, [open]);

  if (book === undefined) {
    return (
      <button type="button" className={className} onClick={actions.page}>
        {children}
      </button>
    );
  }

  const items: [key: string, run: () => void][] = [
    ["new-page", actions.page],
    ["new-book", book],
  ];
  const choose = (run: () => void) => {
    setOpen(false);
    run();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    const entries = [...(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
    const at = entries.indexOf(document.activeElement as HTMLElement);
    const targets: Record<string, number> = {
      ArrowDown: at + 1,
      ArrowUp: at - 1,
      Home: 0,
      End: entries.length - 1,
    };
    const target = targets[event.key];
    if (target !== undefined) {
      event.preventDefault();
      entries[(target + entries.length) % entries.length]?.focus();
    } else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      setOpen(false);
      menu.current?.parentElement?.querySelector<HTMLElement>("button")?.focus();
    }
  };

  return (
    <span className="vl-menu-anchor vl-new-anchor">
      <button
        type="button"
        className={className}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => {
          setOpen(!open);
        }}
      >
        {children}
      </button>
      {open && (
        <div
          className="vl-menu vl-new-menu"
          role="menu"
          aria-label={t("new-menu")}
          tabIndex={-1}
          id={id}
          ref={menu}
          onKeyDown={onKeyDown}
        >
          {items.map(([key, run]) => (
            <div
              key={key}
              role="menuitem"
              tabIndex={-1}
              onClick={() => {
                choose(run);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  choose(run);
                }
              }}
            >
              {t(key)}
            </div>
          ))}
        </div>
      )}
    </span>
  );
}
