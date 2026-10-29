// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useNavigate } from "react-router";

import { ViewTitle } from "../help/help-link";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { useLoad } from "./context";
import { defaultFolder, NewCollectionPanel } from "./csv-panels";

/** Data: every collection the user can read. */
export function CollectionsIndexView() {
  const t = useT();
  const list = useLoad("collections", (store) => store.list());
  const collections = list.data?.collections;
  const [creating, setCreating] = useState(false);
  const navigate = useNavigate();
  return (
    <div className="vl-view">
      <div className="vl-collection-head">
        <ViewTitle title={t("nav-data")} help="collection.create" />
        <button
          type="button"
          className="vl-filled-button"
          onClick={() => {
            setCreating(true);
          }}
        >
          <Icon name="add" size={20} />
          {t("collection-new")}
        </button>
      </div>
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
      {creating && (
        <NewCollectionPanel
          folder={defaultFolder(collections?.map((c) => c.path) ?? [])}
          onCreated={(id) => {
            setCreating(false);
            void navigate(`/data/${id}`);
          }}
          onClose={() => {
            setCreating(false);
          }}
        />
      )}
    </div>
  );
}
