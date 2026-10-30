// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link } from "react-router";

import { highlight } from "../data/search";
import type { Fragment, PageMeta } from "../data/types";
import { useT } from "../i18n/i18n";

/** Text with its matched runs in `<mark>`. */
export function Marked({
  fragments,
}: {
  readonly fragments: readonly Fragment[];
}) {
  return (
    <>
      {fragments.map((fragment, index) =>
        fragment.hit ? (
          // Runs never reorder, so the index is a stable key.

          <mark key={index}>{fragment.text}</mark>
        ) : (
          <span key={index}>{fragment.text}</span>
        ),
      )}
    </>
  );
}

/** One item: a page, and the text around the match when the body matched. */
interface Item {
  readonly page: PageMeta;
  readonly snippet?: string;
  /** The snippet with the server's marked runs. */
  readonly fragments?: readonly Fragment[] | undefined;
}

/**
 * A list of pages with an optional snippet each. With `query`, the words
 * that matched are marked in titles and snippets (R-SR-1).
 */
export function PageList({
  items,
  empty,
  query,
}: {
  readonly items: readonly Item[];
  readonly empty: string;
  readonly query?: string;
}) {
  const t = useT();
  if (items.length === 0) {
    return <p className="vl-muted">{empty}</p>;
  }
  const marks = (text: string) =>
    query === undefined ? [{ text, hit: false }] : highlight(text, query);
  return (
    <ul className="vl-page-list">
      {items.map(({ page, snippet, fragments }) => (
        <li key={page.id}>
          <Link to={`/p/${page.id}`} className="vl-page-card">
            <span className="vl-page-card-icon" aria-hidden="true">
              {page.icon}
            </span>
            <span>
              <strong>
                {page.title === "" ? (
                  t("page-untitled")
                ) : (
                  <Marked fragments={marks(page.title)} />
                )}
              </strong>
              {snippet !== undefined && snippet !== "" && (
                <small>
                  <Marked
                    fragments={
                      fragments?.some((each) => each.hit) === true
                        ? fragments
                        : marks(snippet)
                    }
                  />
                </small>
              )}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
