// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import { Failure, useAction, when } from "./actions";
import type { BookInfo, BooksApi, HistoryEntry } from "./api";
import { outlineRows } from "./outline";

/** What changed in one history entry against the book now. */
function OutlineChanges({
  entry,
  info,
}: {
  readonly entry: HistoryEntry;
  readonly info: BookInfo;
}) {
  const t = useT();
  const now = new Map(
    outlineRows(info.toc).map((row) => [row.id, row.node.label]),
  );
  return (
    <ol className="vl-book-outline">
      {entry.outline.map((line) => {
        const current = now.get(line.id);
        const changed = current !== line.label;
        return (
          <li
            key={line.id}
            style={{ paddingInlineStart: `${String((line.depth - 1) * 14)}px` }}
          >
            <span>{line.label}</span>
            {changed && (
              <small className="vl-book-change">
                {current === undefined
                  ? t("books-history-gone")
                  : t("books-history-now", { label: current })}
              </small>
            )}
          </li>
        );
      })}
    </ol>
  );
}

/** The outline's history: every reorder, nest, add and remove, newest first. */
export function HistoryPanel({
  api,
  info,
  canEdit,
  version,
  onChanged,
}: {
  readonly api: BooksApi;
  readonly info: BookInfo;
  readonly canEdit: boolean;
  readonly version: number;
  readonly onChanged: () => Promise<void>;
}) {
  const t = useT();
  const loaded = useLoaded(
    () => api.history(info.id),
    [api, info.id, version],
    failureText,
  );
  const [open, setOpen] = useState<string>();
  const { busy, error, run } = useAction();
  return (
    <section className="vl-book-panel" aria-labelledby="vl-book-history-title">
      <h2 id="vl-book-history-title">{t("books-history")}</h2>
      <Ready loaded={loaded}>
        {(entries) =>
          entries.length === 0 ? (
            <p className="vl-muted">{t("books-history-empty")}</p>
          ) : (
            <ul className="vl-book-entries">
              {entries.map((entry) => (
                <li key={entry.id}>
                  <button
                    type="button"
                    className="vl-text-button"
                    aria-expanded={open === entry.id}
                    onClick={() => {
                      setOpen((current) =>
                        current === entry.id ? undefined : entry.id,
                      );
                    }}
                  >
                    {t("books-history-entry", {
                      op: t(`books-op-${entry.op.replaceAll("_", "-")}`),
                      when: when(entry.at),
                    })}
                  </button>
                  {open === entry.id && (
                    <div className="vl-book-entry">
                      <OutlineChanges entry={entry} info={info} />
                      {canEdit && (
                        <button
                          type="button"
                          className="vl-filled-button"
                          disabled={busy}
                          onClick={() => {
                            run(async () => {
                              await api.restoreOutline(info.id, entry.id);
                              await onChanged();
                            });
                          }}
                        >
                          {t("books-history-restore")}
                        </button>
                      )}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )
        }
      </Ready>
      <Failure error={error} />
    </section>
  );
}
