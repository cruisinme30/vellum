// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, type KeyboardEvent } from "react";
import { useNavigate } from "react-router";

import type { TreeMove } from "../data/tree";
import type { PageMeta } from "../data/types";
import { useServices } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import { Icon } from "./icon";

interface PageMenuProperties {
  readonly page: PageMeta;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onRename: () => void;
  readonly onMove: (how: TreeMove) => void;
  readonly onAddChild: () => void;
}

/** The ⋯ menu of a tree row (WAI-ARIA menu button). */
export function PageMenu({
  page,
  open,
  onOpenChange,
  onRename,
  onMove,
  onAddChild,
}: PageMenuProperties) {
  const t = useT();
  const id = useId();
  const navigate = useNavigate();
  const { store } = useServices();
  const menu = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) {
      menu.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    }
  }, [open]);

  const items: [key: string, icon: string, run: () => void][] = [
    ["menu-add-child", "add", onAddChild],
    ["menu-rename", "edit", onRename],
    [
      page.starred ? "menu-unstar" : "menu-star",
      page.starred ? "star-fill" : "star",
      () => {
        store.toggleStar(page.id);
      },
    ],
    [
      "menu-move-up",
      "arrow_upward",
      () => {
        onMove("up");
      },
    ],
    [
      "menu-move-down",
      "arrow_downward",
      () => {
        onMove("down");
      },
    ],
    [
      "menu-indent",
      "format_indent_increase",
      () => {
        onMove("indent");
      },
    ],
    [
      "menu-outdent",
      "format_indent_decrease",
      () => {
        onMove("outdent");
      },
    ],
    [
      "menu-trash",
      "delete",
      () => {
        store.trashPage(page.id);
        void navigate("/");
      },
    ],
  ];

  const onKeyDown = (event: KeyboardEvent) => {
    event.stopPropagation();
    const entries = [
      ...(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ??
        []),
    ];
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
      onOpenChange(false);
    }
  };

  return (
    <span className="vl-menu-anchor">
      <button
        type="button"
        className="vl-icon-button vl-tree-more"
        tabIndex={-1}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-label={t("menu-label", {
          title: page.title === "" ? t("page-untitled") : page.title,
        })}
        onClick={(event) => {
          event.stopPropagation();
          onOpenChange(!open);
        }}
      >
        <Icon name="more_horiz" size={20} />
      </button>
      {open && (
        <div
          className="vl-menu"
          role="menu"
          tabIndex={-1}
          id={id}
          ref={menu}
          onKeyDown={onKeyDown}
        >
          {items.map(([key, icon, run]) => (
            <div
              key={key}
              role="menuitem"
              tabIndex={-1}
              onClick={(event) => {
                event.stopPropagation();
                onOpenChange(false);
                run();
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onOpenChange(false);
                  run();
                }
              }}
            >
              <Icon name={icon} size={20} />
              {t(key)}
            </div>
          ))}
        </div>
      )}
    </span>
  );
}
