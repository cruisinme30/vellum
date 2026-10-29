// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { NodeViewContent, NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";

import { useT } from "../../i18n/i18n";
import { Icon } from "../../shell/icon";

/** `::: tabs`: an ARIA tab list over the `tab` children, which stay editable. */
export function TabsView({ node, editor, getPos }: ReactNodeViewProps) {
  const t = useT();
  const prefix = useId();
  const tabId = (index: number) => `${prefix}-tab-${String(index)}`;
  const panelId = (index: number) => `${prefix}-panel-${String(index)}`;
  const wrapper = useRef<HTMLDivElement>(null);
  const titles: string[] = [];
  for (let index = 0; index < node.childCount; index += 1) {
    titles.push(String(node.child(index).attrs["title"]));
  }
  const [chosen, setChosen] = useState(0);
  const active = Math.min(chosen, titles.length - 1);

  useEffect(() => {
    const panels = wrapper.current?.querySelectorAll<HTMLElement>(":scope .vl-tab-panels .vl-tab");
    for (const [index, panel] of [...(panels ?? [])].entries()) {
      panel.hidden = index !== active;
      panel.id = panelId(index);
      panel.setAttribute("aria-labelledby", tabId(index));
    }
  });

  const focusTab = (index: number) => {
    const next = (index + titles.length) % titles.length;
    setChosen(next);
    wrapper.current?.querySelector<HTMLElement>("#" + CSS.escape(tabId(next)))?.focus();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    const moves: Record<string, number> = {
      ArrowRight: active + 1,
      ArrowLeft: active - 1,
      Home: 0,
      End: titles.length - 1,
    };
    const target = moves[event.key];
    if (target !== undefined) {
      event.preventDefault();
      focusTab(target);
    }
  };
  const addTab = () => {
    const pos = getPos();
    if (pos === undefined) {
      return;
    }
    const end = pos + node.nodeSize - 1;
    editor
      .chain()
      .insertContentAt(end, {
        type: "tab",
        attrs: { title: t("tabs-new-title", { n: titles.length + 1 }) },
        content: [{ type: "paragraph" }],
      })
      .run();
    setChosen(titles.length);
  };

  return (
    <NodeViewWrapper className="vl-block vl-tabs" ref={wrapper}>
      <div
        className="vl-tab-list"
        role="tablist"
        aria-label={t("tabs-label")}
        contentEditable={false}
      >
        {titles.map((title, index) => (
          <button
            key={`${String(index)}-${title}`}
            id={tabId(index)}
            type="button"
            role="tab"
            aria-selected={index === active}
            aria-controls={panelId(index)}
            tabIndex={index === active ? 0 : -1}
            onClick={() => {
              setChosen(index);
            }}
            onKeyDown={onKeyDown}
          >
            {title === "" ? t("tabs-untitled") : title}
          </button>
        ))}
        {editor.isEditable && (
          <button
            type="button"
            className="vl-icon-button"
            aria-label={t("tabs-add")}
            onClick={addTab}
          >
            <Icon name="add" />
          </button>
        )}
      </div>
      <NodeViewContent className="vl-tab-panels" />
    </NodeViewWrapper>
  );
}

/** One tab panel, with its title editable in place. */
export function TabView({ node, updateAttributes, editor }: ReactNodeViewProps) {
  const t = useT();
  return (
    <NodeViewWrapper className="vl-tab" role="tabpanel">
      <input
        className="vl-inline-input vl-tab-title"
        contentEditable={false}
        aria-label={t("tabs-title")}
        value={String(node.attrs["title"])}
        readOnly={!editor.isEditable}
        onChange={(event) => {
          updateAttributes({ title: event.target.value });
        }}
      />
      <NodeViewContent />
    </NodeViewWrapper>
  );
}
