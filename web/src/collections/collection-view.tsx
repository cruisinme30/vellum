// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useId, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { BoardView } from "./board-view";
import { CalendarView } from "./calendar-view";
import { GalleryView, ListView } from "./card-views";
import { messageOf, useCollections, useLoad, type Loaded } from "./context";
import { RowPanel } from "./row-panel";
import { TableView } from "./table-view";
import { TimelineView } from "./timeline-view";
import type { CollectionInfo, ViewKind, ViewResult } from "./types";
import type { ViewProperties } from "./view-properties";
import { tabId, ViewTabs } from "./view-tabs";

const LAYOUTS: Readonly<Record<ViewKind, (properties: ViewProperties) => React.ReactNode>> = {
  table: TableView,
  board: BoardView,
  gallery: GalleryView,
  calendar: CalendarView,
  timeline: TimelineView,
  list: ListView,
};

function Loading() {
  const t = useT();
  return (
    <p className="vl-muted" role="status">
      {t("collection-loading")}
    </p>
  );
}

/** Where a collection's results go: the chosen layout, or why there is none. */
function ViewResultPanel({
  info,
  result,
  onOpen,
}: {
  readonly info: CollectionInfo;
  readonly result: Loaded<ViewResult>;
  readonly onOpen: (rowId: string) => void;
}) {
  if (result.error !== undefined) {
    return (
      <p className="vl-field-error" role="alert">
        {result.error}
      </p>
    );
  }
  if (!result.data) {
    return <Loading />;
  }
  const Layout = LAYOUTS[result.data.kind];
  return <Layout info={info} result={result.data} onOpen={onOpen} />;
}

/** A loaded collection: view tabs, the chosen view, "New row" and the row panel. */
function CollectionBody({ collection }: { readonly collection: CollectionInfo }) {
  const t = useT();
  const idBase = useId();
  const panelId = useId();
  const store = useCollections();
  const [parameters, setParameters] = useSearchParams();
  const viewName = parameters.get("view") ?? undefined;
  const result = useLoad(`query:${collection.id}:${viewName ?? ""}`, (s) =>
    s.query({
      collection_id: collection.id,
      ...(viewName === undefined ? {} : { view: viewName }),
    }),
  );
  const [open, setOpen] = useState<{ id: string; fresh: boolean }>();
  const [createError, setCreateError] = useState<string>();
  const close = useCallback(() => {
    setOpen(undefined);
  }, []);
  const openRow = useCallback((id: string) => {
    setOpen({ id, fresh: false });
  }, []);
  const activeName = (result.data?.view ?? viewName ?? "").toLowerCase();
  const active = Math.max(
    0,
    collection.views.findIndex((v) => v.name.toLowerCase() === activeName),
  );
  const newRow = async () => {
    try {
      const written = await store.createRow({ collection_id: collection.id });
      setCreateError(undefined);
      setOpen(written.row ? { id: written.row.row.id, fresh: true } : undefined);
    } catch (error) {
      setCreateError(messageOf(error));
    }
  };

  return (
    <div className="vl-collection">
      <nav aria-label={t("breadcrumbs")} className="vl-breadcrumbs">
        <ol>
          <li>
            <Link to="/data">{t("nav-data")}</Link>
          </li>
          <li aria-current="page">{collection.title}</li>
        </ol>
      </nav>
      <div className="vl-collection-head">
        <h1>{collection.title}</h1>
        <button type="button" className="vl-filled-button" onClick={() => void newRow()}>
          <Icon name="add" size={20} />
          {t("collection-new-row")}
        </button>
      </div>
      {createError !== undefined && (
        <p className="vl-field-error" role="alert">
          {createError}
        </p>
      )}
      <ViewTabs
        views={collection.views}
        active={active}
        idBase={idBase}
        panelId={panelId}
        onSelect={(name) => {
          setParameters({ view: name });
        }}
      />
      <div
        id={panelId}
        role="tabpanel"
        className="vl-view-panel"
        aria-labelledby={tabId(idBase, active)}
      >
        <ViewResultPanel info={collection} result={result} onOpen={openRow} />
      </div>
      {open && (
        <RowPanel
          key={open.id}
          info={collection}
          rowId={open.id}
          focusTitle={open.fresh}
          onClose={close}
        />
      )}
    </div>
  );
}

/** One collection, at `/data/:collectionId?view=<name>`. */
export function CollectionView() {
  const t = useT();
  const { collectionId = "" } = useParams();
  const info = useLoad(`info:${collectionId}`, (s) => s.info(collectionId));
  if (info.error !== undefined) {
    return (
      <div className="vl-view">
        <h1>{t("collection-missing")}</h1>
        <p className="vl-muted">{info.error}</p>
        <p>
          <Link to="/data">{t("collection-back")}</Link>
        </p>
      </div>
    );
  }
  return info.data ? <CollectionBody collection={info.data} /> : <Loading />;
}
