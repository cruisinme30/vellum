// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { useSession } from "../api/session";
import { usePageAssistant } from "../assist/page-assistant";
import { PageFilesProvider } from "../attachments/context";
import { FilesPanel } from "../attachments/files-panel";
import { useServerApis, useServerApi } from "../data/server-apis";
import { ancestors, siblings } from "../data/tree";
import type { PageMeta } from "../data/types";
import { useServices, useWorkspace } from "../data/workspace-context";
import type { ChipSources } from "../editor/chips/items";
import { PageEditor, type EditorControls } from "../editor/page-editor";
import { useT } from "../i18n/i18n";
import { useShare } from "../sharing/share-button";
import { Icon } from "../shell/icon";
import { ExportButton } from "../transfer/export-button";
import { issueChipStatus } from "../work/api";
import { usePageTask } from "../work/quick-add";

import { HistoryPanel } from "./history-panel";
import { NotFoundView } from "./info";
import { PageList } from "./page-list";
import { ReadingView, usePageViewMode } from "./reading-view";

const CARET_COLORS = ["#b3261e", "#00658f", "#386a20", "#7d5260", "#4f5b92"];

/** A stable caret colour per person. */
export function caretColor(handle: string): string {
  let sum = 0;
  for (const char of handle) {
    sum += char.codePointAt(0) ?? 0;
  }
  return CARET_COLORS[sum % CARET_COLORS.length] ?? "#4f5b92";
}

/**
 * The History button for the editor's bar and the panel it opens. Both
 * are absent in demo mode, which keeps no versions.
 */
function usePageHistory(pageId: string) {
  const t = useT();
  const { history } = useServices();
  const session = useSession();
  const [openFor, setOpenFor] = useState<string>();
  const [controls, setControls] = useState<EditorControls>();
  const open = openFor === pageId;
  const button = history && (
    <button
      type="button"
      className="vl-icon-button"
      aria-label={t("history-open")}
      title={t("history-open")}
      aria-expanded={open}
      onClick={() => {
        setOpenFor(open ? undefined : pageId);
      }}
    >
      <Icon name="history" />
    </button>
  );
  const panel = history && open && (
    <HistoryPanel
      pageId={pageId}
      history={history}
      meId={session?.status.user?.id}
      canRestore={controls?.canWrite ?? false}
      saveFirst={controls?.save}
      onClose={() => {
        setOpenFor(undefined);
      }}
    />
  );
  // A viewer's title is not theirs to change (#567).
  const readOnly = controls?.canWrite === false;
  return { button, panel, controls, setControls, readOnly };
}

/**
 * The Files button for the editor's bar and the page's files panel (D277).
 * Both are absent in demo mode, which has nowhere to keep files.
 */
function usePageFilesPanel(pageId: string, canWrite: boolean) {
  const t = useT();
  const available = useServerApi("attachments") !== undefined;
  const [openFor, setOpenFor] = useState<string>();
  const open = openFor === pageId;
  const button = available && (
    <button
      type="button"
      className="vl-icon-button"
      aria-label={t("files-open")}
      title={t("files-open")}
      aria-expanded={open}
      onClick={() => {
        setOpenFor(open ? undefined : pageId);
      }}
    >
      <Icon name="attach_file" />
    </button>
  );
  const panel = available && open && (
    <FilesPanel
      canWrite={canWrite}
      onClose={() => {
        setOpenFor(undefined);
      }}
    />
  );
  return { button, panel };
}

/**
 * People, pages and issues for the editor's chip menus (#93). The object
 * lives as long as the Work API; it reads the latest workspace each time.
 */
function useChipSources(): ChipSources {
  const workspace = useWorkspace();
  const navigate = useNavigate();
  const { work } = useServerApis();
  const latest = useRef(workspace);
  useEffect(() => {
    latest.current = workspace;
  }, [workspace]);
  return useMemo(
    () => ({
      people: () => latest.current.people,
      pages: () => latest.current.pages.filter((page) => page.folder !== true),
      ...(work === undefined
        ? {}
        : {
            issues: async (query: string) => {
              const found = await work.searchIssues(query);
              return found.map(({ key, title }) => ({ key, title }));
            },
            issueStatus: issueChipStatus(work),
            openIssue: (key: string) => {
              void navigate(`/issues/${encodeURIComponent(key)}`);
            },
          }),
    }),
    [work, navigate],
  );
}

/**
 * Whether the page `pageId` may still arrive: the workspace is `loading`,
 * or the store is being asked for it. A page shared on its own is in none
 * of the person's spaces' trees (#567), so when the loaded workspace lacks
 * it, the store fetches it by ID; it joins the workspace if the person may
 * open it. False once it is `known` or known to be missing.
 */
function useOutsidePage(pageId: string, known: boolean, loading: boolean): boolean {
  const { store } = useServices();
  const absent = !known && !loading;
  const [missing, setMissing] = useState<string>();
  useEffect(() => {
    if (!absent || store.loadPage === undefined) return;
    let live = true;
    void store.loadPage(pageId).then((found) => {
      if (live && found === undefined) setMissing(pageId);
    });
    return () => {
      live = false;
    };
  }, [absent, store, pageId]);
  if (known || loading) return loading;
  return store.loadPage !== undefined && missing !== pageId;
}

/** A page: breadcrumbs, title, properties and the editor. */
export function PageView() {
  const t = useT();
  const { pageId = "" } = useParams();
  const navigate = useNavigate();
  const { store, collab } = useServices();
  const workspace = useWorkspace();
  const pageHistory = usePageHistory(pageId);
  const assistant = usePageAssistant(pageId);
  const { work, drive } = useServerApis();
  const view = usePageViewMode();
  const share = useShare(pageId);
  const chips = useChipSources();
  const pageFiles = usePageFilesPanel(pageId, pageHistory.controls?.canWrite ?? false);
  const page = workspace.pages.find((p) => p.id === pageId);
  const waiting = useOutsidePage(pageId, page !== undefined, workspace.loading === true);
  const onSelectionTask = usePageTask(page);
  const trail = useMemo(() => ancestors(workspace.pages, pageId), [workspace.pages, pageId]);
  const space = workspace.spaces.find((s) => s.id === page?.spaceId);
  const pages = workspace.pages;
  const resolvePage = useCallback(
    (target: string) => {
      const id = target.startsWith("vellum://doc/")
        ? target.slice("vellum://doc/".length)
        : undefined;
      const found = pages.find((p) =>
        id ? p.id === id : p.title.toLowerCase() === target.toLowerCase(),
      );
      return found ? { id: found.id, title: found.title } : undefined;
    },
    [pages],
  );
  const onOpenPage = useCallback(
    (id: string) => {
      void navigate(`/p/${id}`);
    },
    [navigate],
  );
  const user = useMemo(
    () => ({ name: workspace.me.name, color: caretColor(workspace.me.handle) }),
    [workspace.me],
  );

  if (!page) {
    return waiting ? (
      <p className="vl-muted" role="status">
        {t("workspace-loading")}
      </p>
    ) : (
      <NotFoundView />
    );
  }
  return (
    <article className="vl-page">
      <nav aria-label={t("breadcrumbs")} className="vl-breadcrumbs">
        <ol>
          <li>{space?.name}</li>
          {trail.map((p) => (
            <li key={p.id}>
              <Link to={`/p/${p.id}`}>{p.title === "" ? t("page-untitled") : p.title}</Link>
            </li>
          ))}
        </ol>
      </nav>
      <header className="vl-page-head">
        <span className="vl-page-icon" aria-hidden="true">
          {page.icon}
        </span>
        <h1 className="vl-page-title">
          <input
            key={page.id}
            className="vl-title-input"
            aria-label={t("page-title")}
            placeholder={t("page-untitled")}
            defaultValue={page.title}
            readOnly={pageHistory.readOnly}
            onBlur={(event) => {
              const title = event.currentTarget.value.trim();
              if (title !== page.title) {
                store.renamePage(page.id, title);
              }
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                event.currentTarget.blur();
              }
            }}
          />
        </h1>
        <button
          type="button"
          className="vl-icon-button"
          aria-pressed={page.starred}
          aria-label={t("page-star")}
          onClick={() => {
            store.toggleStar(page.id);
          }}
        >
          <Icon name={page.starred ? "star-fill" : "star"} />
        </button>
        {share.button}
      </header>
      {share.panel}
      {page.folder === true ? (
        <FolderContents pages={pages} folder={page} />
      ) : (
        <>
          <PageFacts page={page} />
          {view.toggle}
          {view.reading && view.render !== undefined ? (
            <ReadingView pageId={page.id} api={view.render} />
          ) : (
            <PageFilesProvider pageId={page.id}>
              <PageEditor
                pageId={page.id}
                spaceId={page.spaceId}
                seed={store.seedFor(page.id)}
                connector={collab}
                user={user}
                resolvePage={resolvePage}
                onOpenPage={onOpenPage}
                onControls={pageHistory.setControls}
                work={work}
                me={workspace.me.handle}
                chips={chips}
                scanSecrets={drive?.scanSecrets}
                onSelectionTask={onSelectionTask}
                actions={
                  <>
                    {assistant.button}
                    <ExportButton target={{ kind: "page", id: page.id, title: page.title }} />
                    {pageFiles.button}
                    {pageHistory.button}
                  </>
                }
              />
              {assistant.panel}
              {pageFiles.panel}
            </PageFilesProvider>
          )}
          {pageHistory.panel}
        </>
      )}
    </article>
  );
}

/** Status, owner and last update of a page. */
function PageFacts({ page }: { readonly page: PageMeta }) {
  const t = useT();
  const workspace = useWorkspace();
  const owner = workspace.people.find((p) => p.handle === page.owner);
  const updated = new Date(page.updatedAt).toLocaleDateString("en", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  return (
    <dl className="vl-page-props">
      <div>
        <dt>{t("page-status")}</dt>
        <dd>
          <span className={`vl-chip vl-status-${page.status}`}>{t(`status-${page.status}`)}</span>
        </dd>
      </div>
      <div>
        <dt>{t("page-owner")}</dt>
        <dd>{owner?.name ?? page.owner}</dd>
      </div>
      <div>
        <dt>{t("page-updated")}</dt>
        <dd>
          <time dateTime={page.updatedAt}>{updated}</time>
        </dd>
      </div>
    </dl>
  );
}

/** A drive folder in place of a page body: the pages in it. */
function FolderContents({
  pages,
  folder,
}: {
  readonly pages: readonly PageMeta[];
  readonly folder: PageMeta;
}) {
  const t = useT();
  return (
    <section>
      <p className="vl-muted">{t("folder-intro")}</p>
      <PageList
        items={siblings(pages, folder.spaceId, folder.id).map((child) => ({ page: child }))}
        empty={t("folder-empty")}
      />
    </section>
  );
}
