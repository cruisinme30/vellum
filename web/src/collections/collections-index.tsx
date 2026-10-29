// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link } from "react-router";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { useLoad } from "./context";

/** Data: every collection the user can read. */
export function CollectionsIndexView() {
  const t = useT();
  const list = useLoad("collections", (store) => store.list());
  const collections = list.data?.collections;
  return (
    <div className="vl-view">
      <h1>{t("nav-data")}</h1>
      <p className="vl-muted">{t("collections-intro")}</p>
      {list.error !== undefined && (
        <p className="vl-field-error" role="alert">
          {list.error}
        </p>
      )}
      {list.error === undefined && !collections && (
        <p className="vl-muted" role="status">
          {t("collection-loading")}
        </p>
      )}
      {collections?.length === 0 && <p className="vl-muted">{t("collections-empty")}</p>}
      {collections !== undefined && collections.length > 0 && (
        <ul className="vl-page-list">
          {collections.map((collection) => (
            <li key={collection.id}>
              <Link className="vl-page-card" to={`/data/${collection.id}`}>
                <span className="vl-page-card-icon" aria-hidden="true">
                  <Icon name="table_view" />
                </span>
                <span>
                  <strong>{collection.title}</strong>{" "}
                  <small>
                    {t("collections-summary", {
                      rows: collection.rows,
                      views: collection.views.length,
                    })}
                  </small>
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
