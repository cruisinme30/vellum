// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import { Failure, formText, useAction, when, type Submit } from "./actions";
import type { BookDiff, BooksApi } from "./api";

function DiffView({ diff }: { readonly diff: BookDiff }) {
  const t = useT();
  const empty =
    diff.added.length + diff.removed.length + diff.moved.length + diff.changed.length === 0 &&
    diff.settings.length === 0;
  if (empty) return <p className="vl-muted">{t("books-diff-none")}</p>;
  return (
    <div className="vl-book-diff">
      {diff.added.length > 0 && (
        <>
          <h3>{t("books-diff-added")}</h3>
          <ul>
            {diff.added.map((s) => (
              <li key={s.id}>{s.label}</li>
            ))}
          </ul>
        </>
      )}
      {diff.removed.length > 0 && (
        <>
          <h3>{t("books-diff-removed")}</h3>
          <ul>
            {diff.removed.map((s) => (
              <li key={s.id}>{s.label}</li>
            ))}
          </ul>
        </>
      )}
      {diff.moved.length > 0 && (
        <>
          <h3>{t("books-diff-moved")}</h3>
          <ul>
            {diff.moved.map((s) => (
              <li key={s.id}>{t("books-diff-moved-line", { from: s.from, to: s.to })}</li>
            ))}
          </ul>
        </>
      )}
      {diff.changed.length > 0 && (
        <>
          <h3>{t("books-diff-changed")}</h3>
          {diff.changed.map((s) => (
            <details key={s.id}>
              <summary>
                {t("books-diff-changed-line", {
                  label: s.label,
                  added: s.added,
                  removed: s.removed,
                })}
              </summary>
              <pre className="vl-book-patch">{s.patch}</pre>
            </details>
          ))}
        </>
      )}
      {diff.settings.length > 0 && (
        <>
          <h3>{t("books-diff-settings")}</h3>
          <ul>
            {diff.settings.map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

const NOW = "";

/** Tagged snapshots of the whole book, and what changed between two of them. */
export function SnapshotsPanel({
  api,
  book,
  canEdit,
}: {
  readonly api: BooksApi;
  readonly book: string;
  readonly canEdit: boolean;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const loaded = useLoaded(() => api.snapshots(book), [api, book, version], failureText);
  const [compare, setCompare] = useState<{ from: string; to: string }>();
  const diff = useLoaded(
    async () => {
      if (compare === undefined) return;
      const to = compare.to === NOW ? undefined : compare.to;
      return api.diff(book, compare.from, to);
    },
    [api, book, compare?.from, compare?.to],
    failureText,
  );
  const { busy, error, run } = useAction();
  const take: Submit = (event) => {
    event.preventDefault();
    const tag = formText(event.currentTarget, "tag");
    const message = formText(event.currentTarget, "message");
    const form = event.currentTarget;
    if (tag === "") return;
    run(async () => {
      await api.createSnapshot(book, tag, message);
      form.reset();
      setVersion((v) => v + 1);
    });
  };
  const choose: Submit = (event) => {
    event.preventDefault();
    setCompare({
      from: formText(event.currentTarget, "from"),
      to: formText(event.currentTarget, "to"),
    });
  };
  return (
    <section className="vl-book-panel" aria-labelledby="vl-book-snapshots-title">
      <h2 id="vl-book-snapshots-title">{t("books-snapshots")}</h2>
      {canEdit && (
        <form className="vl-book-tool-form" onSubmit={take}>
          <label>
            {t("books-snapshot-tag")}
            <input
              name="tag"
              required
              pattern="[A-Za-z0-9._\-]+"
              placeholder="v1.0"
              autoComplete="off"
            />
          </label>
          <label>
            {t("books-snapshot-message")}
            <input name="message" autoComplete="off" />
          </label>
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("books-snapshot-take")}
          </button>
        </form>
      )}
      <Failure error={error} />
      <Ready loaded={loaded}>
        {(snapshots) =>
          snapshots.length === 0 ? (
            <p className="vl-muted">{t("books-snapshots-empty")}</p>
          ) : (
            <>
              <ul className="vl-book-entries">
                {snapshots.map((s) => (
                  <li key={s.tag}>
                    <strong>{s.tag}</strong>{" "}
                    <small className="vl-muted">{when(s.created_at)}</small>
                    {s.message !== "" && <div>{s.message}</div>}
                  </li>
                ))}
              </ul>
              <form className="vl-book-compare" onSubmit={choose}>
                <label>
                  {t("books-diff-from")}
                  <select name="from" defaultValue={snapshots[0]?.tag}>
                    {snapshots.map((s) => (
                      <option key={s.tag} value={s.tag}>
                        {s.tag}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  {t("books-diff-to")}
                  <select name="to" defaultValue={NOW}>
                    <option value={NOW}>{t("books-diff-now")}</option>
                    {snapshots.map((s) => (
                      <option key={s.tag} value={s.tag}>
                        {s.tag}
                      </option>
                    ))}
                  </select>
                </label>
                <button type="submit" className="vl-filled-button">
                  {t("books-diff-compare")}
                </button>
              </form>
            </>
          )
        }
      </Ready>
      {compare !== undefined && (
        <Ready loaded={diff}>{(found) => found !== undefined && <DiffView diff={found} />}</Ready>
      )}
    </section>
  );
}
