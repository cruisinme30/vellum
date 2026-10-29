// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { NodeViewRenderer } from "@tiptap/core";

import type { Translate } from "../../i18n/i18n";

import type { ChipSources, IssueStatus } from "./items";

/** How long a looked-up status is reused before Work is asked again. */
const FRESH_MS = 60_000;

/**
 * Caches issue status lookups for every chip on the page, so ten chips of
 * one issue make one request, and a chip re-rendered by an edit does not
 * ask again within a minute.
 */
export function statusCache(
  lookup: (key: string) => Promise<IssueStatus | undefined>,
  now: () => number = Date.now,
) {
  const entries = new Map<string, { at: number; value: Promise<IssueStatus | undefined> }>();
  return (key: string): Promise<IssueStatus | undefined> => {
    const hit = entries.get(key);
    if (hit && now() - hit.at < FRESH_MS) return hit.value;
    const value = lookup(key).then(
      (status) => status,
      // A failed lookup leaves the chip as a bare key.
      (): IssueStatus | undefined => undefined,
    );
    entries.set(key, { at: now(), value });
    return value;
  };
}

/**
 * A node view for issue chips with live status (#93): the key, then the
 * state once Work answers, with the title as the accessible description.
 * A click opens the issue. The Markdown stays the bare key.
 */
export function issueChipView(sources: ChipSources, t: Translate): NodeViewRenderer {
  const lookup = sources.issueStatus ? statusCache(sources.issueStatus) : undefined;
  return ({ node }) => {
    const key = String(node.attrs["key"]);
    const dom = document.createElement("span");
    dom.className = "vl-chip vl-issue";
    dom.dataset["type"] = "issueKey";
    dom.dataset["key"] = key;
    dom.contentEditable = "false";
    const label = document.createElement("span");
    label.textContent = key;
    dom.append(label);
    if (sources.openIssue) {
      const open = sources.openIssue;
      dom.setAttribute("role", "link");
      dom.tabIndex = -1;
      dom.addEventListener("click", (event) => {
        event.preventDefault();
        open(key);
      });
    }
    void lookup?.(key).then((status) => {
      if (!status) return;
      const state = document.createElement("span");
      state.className = "vl-issue-state";
      state.textContent = status.state;
      dom.append(state);
      dom.dataset["closed"] = String(status.closed);
      dom.title = t("chips-issue-title", { key, title: status.title, state: status.state });
      dom.setAttribute("aria-label", dom.title);
    });
    return { dom };
  };
}
