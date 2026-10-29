// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useState } from "react";

import { useT } from "../i18n/i18n";

import { messageOf, useOptionalCollections } from "./context";
import type { EmbedResult } from "./types";

/** How long the block's text must rest before the preview asks again. */
const SETTLE_MS = 400;
/** Rows a preview shows; the page shows them all. */
const PREVIEW_ROWS = 8;

/** Whether a block names its collection: an `id:` line with a value. */
function hasId(fence: string): boolean {
  return fence.split("\n").some((line) => {
    const trimmed = line.trim();
    return trimmed.startsWith("id:") && trimmed.slice(3).trim() !== "";
  });
}

type State =
  | { readonly fence: string; readonly result: EmbedResult }
  | { readonly fence: string; readonly error: string };

/**
 * The live rows of a `collection` block while it is edited, through
 * `collections.embed`, so the reader's permissions apply (D193). Outside
 * the app's collections provider it shows nothing.
 */
export function CollectionEmbedPreview({ fence }: { readonly fence: string }) {
  const t = useT();
  const store = useOptionalCollections();
  const [state, setState] = useState<State>();
  const [version, setVersion] = useState(0);
  useEffect(
    () =>
      store?.subscribe(() => {
        setVersion((v) => v + 1);
      }),
    [store],
  );
  useEffect(() => {
    if (!store || !hasId(fence)) return;
    let live = true;
    const timer = setTimeout(() => {
      store.embed(fence).then(
        (result) => {
          if (live) setState({ fence, result });
        },
        (error: unknown) => {
          if (live) setState({ fence, error: messageOf(error) });
        },
      );
    }, SETTLE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [store, fence, version]);

  if (!store) return <p className="vl-muted">{t("fence-rendered", { lang: "collection" })}</p>;
  if (!hasId(fence)) {
    return <p className="vl-muted">{t("fence-collection-needs-id")}</p>;
  }
  if (!state) {
    return (
      <p className="vl-muted" role="status">
        {t("collection-loading")}
      </p>
    );
  }
  if ("error" in state) {
    return <p className="vl-field-error">{state.error}</p>;
  }
  const { result, title } = state.result;
  const rows = result.groups.flatMap((g) => g.rows);
  return (
    <figure className="vl-embed-preview">
      <figcaption>
        {t("fence-collection-caption", { title, view: result.view, total: result.total })}
      </figcaption>
      <table className="vl-table">
        <thead>
          <tr>
            {result.columns.map((c) => (
              <th key={c.key} scope="col">
                {c.name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, PREVIEW_ROWS).map((row) => (
            <tr key={row.id}>
              {row.cells.map((cell, index) => (
                <td key={result.columns[index]?.key ?? index}>{cell.display}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  );
}
