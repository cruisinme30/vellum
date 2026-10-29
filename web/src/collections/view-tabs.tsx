// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useRef, type KeyboardEvent } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import type { ViewInfo, ViewKind } from "./types";

const KIND_ICONS: Readonly<Record<ViewKind, string>> = {
  table: "table",
  board: "view_kanban",
  gallery: "grid_view",
  calendar: "calendar_month",
  timeline: "view_timeline",
  list: "view_list",
};

/** The id of a view's tab, so the panel can name it. */
export const tabId = (base: string, index: number) => `${base}-tab-${String(index)}`;

interface ViewTabsProperties {
  readonly views: readonly ViewInfo[];
  readonly active: number;
  /** Prefix of tab ids. */
  readonly idBase: string;
  readonly panelId: string;
  readonly onSelect: (name: string) => void;
}

/** Saved views as tabs; arrows, Home and End move between them and open them. */
export function ViewTabs({ views, active, idBase, panelId, onSelect }: ViewTabsProperties) {
  const t = useT();
  const list = useRef<HTMLDivElement>(null);
  const onKeyDown = (event: KeyboardEvent) => {
    const last = views.length - 1;
    const target = { ArrowLeft: active - 1, ArrowRight: active + 1, Home: 0, End: last }[event.key];
    if (target === undefined) {
      return;
    }
    event.preventDefault();
    const index = target < 0 ? last : target % views.length;
    const view = views[index];
    if (view) {
      onSelect(view.name);
      list.current?.querySelector<HTMLElement>(`#${CSS.escape(tabId(idBase, index))}`)?.focus();
    }
  };
  return (
    <div ref={list} className="vl-view-tabs" role="tablist" aria-label={t("collection-views")}>
      {views.map((view, index) => (
        <button
          key={view.name}
          id={tabId(idBase, index)}
          type="button"
          role="tab"
          className="vl-view-tab"
          aria-selected={index === active}
          aria-controls={panelId}
          tabIndex={index === active ? 0 : -1}
          onClick={() => {
            onSelect(view.name);
          }}
          onKeyDown={onKeyDown}
        >
          <Icon name={KIND_ICONS[view.kind]} size={20} />
          <span>{view.name}</span>
        </button>
      ))}
    </div>
  );
}
