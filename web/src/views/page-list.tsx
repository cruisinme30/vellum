// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link } from "react-router";

import type { PageMeta } from "../data/types";
import { useT } from "../i18n/i18n";

/** A list of pages with an optional snippet each. */
export function PageList({
  items,
  empty,
}: {
  readonly items: readonly { page: PageMeta; snippet?: string }[];
  readonly empty: string;
}) {
  const t = useT();
  if (items.length === 0) {
    return <p className="vl-muted">{empty}</p>;
  }
  return (
    <ul className="vl-page-list">
      {items.map(({ page, snippet }) => (
        <li key={page.id}>
          <Link to={`/p/${page.id}`} className="vl-page-card">
            <span className="vl-page-card-icon" aria-hidden="true">
              {page.icon}
            </span>
            <span>
              <strong>{page.title === "" ? t("page-untitled") : page.title}</strong>
              {snippet !== undefined && snippet !== "" && <small>{snippet}</small>}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
