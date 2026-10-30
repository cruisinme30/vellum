// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useId, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";

import { ViewTitle } from "../help/help-link";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { BoardView } from "./board-view";
import { CalendarView } from "./calendar-view";
import { GalleryView, ListView } from "./card-views";
import { messageOf, useCollections, useLoad, type Loaded } from "./context";
import { ImportPanel } from "./csv-panels";
import { PropertiesPanel } from "./property-editor";
import { RowPanel } from "./row-panel";
import { download } from "./side-panel";
import { TableView } from "./table-view";
import { TimelineView } from "./timeline-view";
import type { CollectionInfo, ViewInfo, ViewKind, ViewResult } from "./types";
import { ViewEditor } from "./view-editor";
import type { ViewProperties } from "./view-properties";
import { tabId, ViewTabs } from "./view-tabs";

/** The side panel open next to the view, if any: one at a time. */
type Panel =
  | { readonly kind: "row"; readonly id: string; readonly fresh: boolean }
  | { readonly kind: "view" | "new-view" | "properties" | "import" };

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

/** The side panel that is open, if any. */
function OpenPanel({
  panel,
  info,
  view,
  onShowView,
  onClose,
}: {
  readonly panel: Panel | undefined;
  readonly info: CollectionInfo;
  readonly view: ViewInfo | undefined;
  readonly onShowView: (name?: string) => void;
  readonly onClose: () => void;
}) {
  switch (panel?.kind) {
    case "row": {
      return (
        <RowPanel
          key={panel.id}
          info={info}
          rowId={panel.id}
          focusTitle={panel.fresh}
          onClose={onClose}
        />
      );
    }
    case "view": {
      return (
        view && (
          <ViewEditor
            key={JSON.stringify(view)}
            info={info}
            view={view}
            onSaved={onShowView}
            onClose={onClose}
          />
        )
      );
    }
    case "new-view": {
      return <ViewEditor info={info} onSaved={onShowView} onClose={onClose} />;
    }
    case "properties": {
      return <PropertiesPanel info={info} onClose={onClose} />;
    }
    case "import": {
      return <ImportPanel info={info} onClose={onClose} />;
    }
    case undefined: {
      return;
    }
  }
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
  const [open, setOpen] = useState<Panel>();
  const [actionError, setActionError] = useState<string>();
  const close = useCallback(() => {
    setOpen(undefined);
  }, []);
  const openRow = useCallback((id: string) => {
    setOpen({ kind: "row", id, fresh: false });
  }, []);
  const activeName = (result.data?.view ?? viewName ?? "").toLowerCase();
  const active = Math.max(
    0,
    collection.views.findIndex((v) => v.name.toLowerCase() === activeName),
  );
  const activeView = collection.views[active];
  const newRow = async () => {
    try {
      const written = await store.createRow({ collection_id: collection.id });
      setActionError(undefined);
      setOpen(written.row ? { kind: "row", id: written.row.row.id, fresh: true } : undefined);
    } catch (error) {
      setActionError(messageOf(error));
    }
  };
  const exportCsv = async () => {
    try {
      const file = await store.exportCsv({
        collection_id: collection.id,
        ...(activeView ? { view: activeView.name } : {}),
      });
      setActionError(undefined);
      download(file.fileName, file.csv);
    } catch (error) {
      setActionError(messageOf(error));
    }
  };
  const showView = (name?: string) => {
    setOpen(undefined);
    setParameters(name === undefined ? {} : { view: name });
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
        <ViewTitle title={collection.title} help="collection.views" />
        <div className="vl-collection-actions">
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setOpen({ kind: "properties" });
            }}
          >
            <Icon name="schema" size={20} />
            {t("collection-properties")}
          </button>
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setOpen({ kind: "import" });
            }}
          >
            <Icon name="arrow_upward" size={20} />
            {t("collection-import")}
          </button>
          <button type="button" className="vl-text-button" onClick={() => void exportCsv()}>
            <Icon name="arrow_downward" size={20} />
            {t("collection-export")}
          </button>
          <button type="button" className="vl-filled-button" onClick={() => void newRow()}>
            <Icon name="add" size={20} />
            {t("collection-new-row")}
          </button>
        </div>
      </div>
      {actionError !== undefined && (
        <p className="vl-field-error" role="alert">
          {actionError}
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
      <div className="vl-view-actions">
        {activeView && (
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setOpen({ kind: "view" });
            }}
          >
            <Icon name="edit" size={20} />
            {t("collection-edit-view-button")}
          </button>
        )}
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            setOpen({ kind: "new-view" });
          }}
        >
          <Icon name="add" size={20} />
          {t("collection-new-view")}
        </button>
      </div>
      <div
        id={panelId}
        role="tabpanel"
        className="vl-view-panel"
        aria-labelledby={tabId(idBase, active)}
      >
        <ViewResultPanel info={collection} result={result} onOpen={openRow} />
      </div>
      <OpenPanel
        panel={open}
        info={collection}
        view={activeView}
        onShowView={showView}
        onClose={close}
      />
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
