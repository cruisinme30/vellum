// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { CopyDiagnostic } from "../shell/copy-diagnostic";
import { ExportButton } from "../transfer/export-button";

import { AccessPanel } from "./access-panel";
import type { BookInfo, BooksApi, BookSummary, Place, TocLine } from "./api";
import { BookTree } from "./book-tree";
import { HistoryPanel } from "./history-panel";
import { MarkdownReader } from "./markdown";
import { isNode, outlineRows, type OutlineRow } from "./outline";
import { PublishPanel } from "./publish-panel";
import { FirstSection, SectionTools } from "./section-tools";
import { SnapshotsPanel } from "./snapshots-panel";

import "./books.css";

const bookPath = (book: string) => `/books/${encodeURIComponent(book)}`;
const sectionPath = (book: string, section: string) =>
  `${bookPath(book)}/${encodeURIComponent(section)}`;

function BookList({ books }: { readonly books: readonly BookSummary[] }) {
  const t = useT();
  if (books.length === 0) return <p className="vl-muted">{t("books-empty")}</p>;
  return (
    <ul className="vl-book-list">
      {books.map((book) => (
        <li key={book.id}>
          <Link to={bookPath(book.id)}>{book.title}</Link>
          <small className="vl-muted">{book.path}</small>
        </li>
      ))}
    </ul>
  );
}

function AllBooks({ api }: { readonly api: BooksApi }) {
  const loaded = useLoaded(() => api.list(), [api], failureText);
  return <Ready loaded={loaded}>{(books) => <BookList books={books} />}</Ready>;
}

/** Books: every book the person can read. */
export function BooksPage() {
  const t = useT();
  return (
    <ServerScreen api="books" help="book.create" title={t("books")} className="vl-books">
      {(api) => <AllBooks api={api} />}
    </ServerScreen>
  );
}

/** The first section that can be opened, depth first. */
export function firstSection(lines: readonly TocLine[]): string | undefined {
  for (const line of lines) {
    if (!isNode(line)) continue;
    if (line.id != undefined && !line.draft) return line.id;
    const inner = firstSection(line.children);
    if (inner !== undefined) return inner;
  }
  return undefined;
}

function TocList({
  book,
  lines,
  current,
}: {
  readonly book: string;
  readonly lines: readonly TocLine[];
  readonly current: string | undefined;
}) {
  return (
    <ol>
      {lines.map((line, index) => {
        if (!isNode(line)) {
          return line.kind === "part" ? (
            <li key={index} className="vl-toc-part">
              {line.title}
            </li>
          ) : (
            <li key={index} className="vl-toc-separator" aria-hidden="true" />
          );
        }
        const label = line.number == undefined ? line.title : `${line.number} ${line.title}`;
        return (
          <li key={line.id ?? index}>
            {line.id == undefined || line.draft ? (
              <span className="vl-muted">{label}</span>
            ) : (
              <Link
                to={sectionPath(book, line.id)}
                aria-current={line.id === current ? "page" : undefined}
              >
                {label}
              </Link>
            )}
            {line.children.length > 0 && (
              <TocList book={book} lines={line.children} current={current} />
            )}
          </li>
        );
      })}
    </ol>
  );
}

type Panel = "history" | "snapshots" | "access" | "publish";

const EDITORS = new Set(["editor", "manager", "owner"]);
const MANAGERS = new Set(["manager", "owner"]);

function SectionView({
  api,
  info,
  section,
  version,
  canEdit,
  hasChildren,
  showAccess,
  onChanged,
}: {
  readonly api: BooksApi;
  readonly info: BookInfo;
  readonly section: string;
  readonly version: number;
  readonly canEdit: boolean;
  readonly hasChildren: boolean;
  readonly showAccess: boolean;
  readonly onChanged: () => Promise<void>;
}) {
  const t = useT();
  const loaded = useLoaded(
    () => api.section(info.id, section),
    [api, info.id, section, version],
    failureText,
  );
  return (
    <Ready loaded={loaded}>
      {(found) => (
        <>
          {canEdit && (
            <SectionTools
              api={api}
              info={info}
              section={found}
              hasChildren={hasChildren}
              onChanged={onChanged}
              sectionPath={sectionPath}
            />
          )}
          {showAccess && (
            <AccessPanel api={api} book={info.id} section={found} onChanged={onChanged} />
          )}
          <article className="vl-book-section">
            <h2>
              {found.number == undefined ? found.title : `${found.number} ${found.title}`}
              {found.restricted && <span className="vl-book-badge">{t("books-restricted")}</span>}
            </h2>
            <ExportButton
              label={t("export-section")}
              target={{
                kind: "book",
                id: info.id,
                title: found.title,
                scope: "subtree",
                section: found.id,
              }}
            />
            <MarkdownReader markdown={found.markdown} title={found.title} />
          </article>
        </>
      )}
    </Ready>
  );
}

const PANELS: readonly {
  readonly key: Panel;
  readonly label: string;
  readonly manager: boolean;
}[] = [
  { key: "history", label: "books-history", manager: false },
  { key: "snapshots", label: "books-snapshots", manager: false },
  { key: "access", label: "books-access", manager: true },
  { key: "publish", label: "books-publish", manager: true },
];

/** The buttons that open the book's panels. */
function BookToolbar({
  panel,
  canManage,
  hasSection,
  onToggle,
}: {
  readonly panel: Panel | undefined;
  readonly canManage: boolean;
  readonly hasSection: boolean;
  readonly onToggle: (panel: Panel) => void;
}) {
  const t = useT();
  const shown = PANELS.filter(
    (p) => (canManage || !p.manager) && (p.key !== "access" || hasSection),
  );
  return (
    <div className="vl-book-toolbar" role="toolbar" aria-label={t("books-book-tools")}>
      {shown.map((p) => (
        <button
          key={p.key}
          type="button"
          className="vl-text-button"
          aria-pressed={panel === p.key}
          onClick={() => {
            onToggle(p.key);
          }}
        >
          {t(p.label)}
        </button>
      ))}
    </div>
  );
}

/** The contents: a tree editors rearrange, or the plain list for readers. */
function Contents({
  api,
  info,
  rows,
  current,
  canEdit,
  onMove,
  onChanged,
}: {
  readonly api: BooksApi;
  readonly info: BookInfo;
  readonly rows: readonly OutlineRow[];
  readonly current: string | undefined;
  readonly canEdit: boolean;
  readonly onMove: (section: string, place: Place) => Promise<void>;
  readonly onChanged: () => Promise<void>;
}) {
  const t = useT();
  let body = <TocList book={info.id} lines={info.toc} current={current} />;
  if (canEdit && rows.length > 0) {
    body = (
      <BookTree
        book={info.id}
        rows={rows}
        current={current}
        maxDepth={info.max_depth}
        onMove={onMove}
        sectionPath={(id) => sectionPath(info.id, id)}
      />
    );
  } else if (canEdit) {
    body = <FirstSection api={api} book={info.id} onChanged={onChanged} />;
  }
  return (
    <nav className="vl-book-toc" aria-label={t("books-contents")}>
      {body}
    </nav>
  );
}

function Book({
  api,
  initial,
  section,
}: {
  readonly api: BooksApi;
  readonly initial: BookInfo;
  readonly section: string | undefined;
}) {
  const t = useT();
  const [info, setInfo] = useState(initial);
  const [version, setVersion] = useState(0);
  const [panel, setPanel] = useState<Panel>();
  const [error, setError] = useState<string>();
  const shown = section ?? firstSection(info.toc);
  const canEdit = EDITORS.has(info.role);
  const canManage = MANAGERS.has(info.role);
  const rows = outlineRows(info.toc);
  const reload = async () => {
    try {
      setInfo(await api.book(info.id));
      setError(undefined);
    } catch (error_: unknown) {
      setError(failureText(error_));
    }
    setVersion((v) => v + 1);
  };
  const onMove = async (id: string, place: Place) => {
    try {
      await api.moveSection(info.id, id, place);
      setError(undefined);
    } catch (error_: unknown) {
      setError(failureText(error_));
    }
    await reload();
  };
  return (
    <>
      <p className="vl-muted">
        <Link to="/books">{t("books")}</Link>
      </p>
      <div className="vl-book-head">
        <h1>{info.title}</h1>
        <BookToolbar
          panel={panel}
          canManage={canManage}
          hasSection={shown !== undefined}
          onToggle={(next) => {
            setPanel((current) => (current === next ? undefined : next));
          }}
        />
      </div>
      <ExportButton
        label={t("export-book")}
        target={{ kind: "book", id: info.id, title: info.title, scope: "whole" }}
      />
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
          <CopyDiagnostic message={error} />
        </p>
      )}
      <div className="vl-book">
        <Contents
          api={api}
          info={info}
          rows={rows}
          current={shown}
          canEdit={canEdit}
          onMove={onMove}
          onChanged={reload}
        />
        <div className="vl-book-main">
          {panel === "history" && (
            <HistoryPanel
              api={api}
              info={info}
              canEdit={canEdit}
              version={version}
              onChanged={reload}
            />
          )}
          {panel === "snapshots" && <SnapshotsPanel api={api} book={info.id} canEdit={canEdit} />}
          {panel === "publish" && canManage && <PublishPanel api={api} book={info.id} />}
          {shown === undefined ? (
            <p className="vl-muted">{t("books-no-sections")}</p>
          ) : (
            <SectionView
              api={api}
              info={info}
              section={shown}
              version={version}
              canEdit={canEdit}
              hasChildren={rows.some((row) => row.parent === shown)}
              showAccess={panel === "access" && canManage}
              onChanged={reload}
            />
          )}
        </div>
      </div>
    </>
  );
}

function BookForApi({
  api,
  book,
  section,
}: {
  readonly api: BooksApi;
  readonly book: string;
  readonly section: string | undefined;
}) {
  const loaded = useLoaded(() => api.book(book), [api, book], failureText);
  return (
    <Ready loaded={loaded}>
      {(info) => <Book key={info.id} api={api} initial={info} section={section} />}
    </Ready>
  );
}

/** One book: its contents beside the section being read. */
export function BookPage() {
  const { bookId = "", sectionId } = useParams();
  return (
    <ServerScreen api="books" className="vl-books">
      {(api) => <BookForApi api={api} book={bookId} section={sectionId} />}
    </ServerScreen>
  );
}
