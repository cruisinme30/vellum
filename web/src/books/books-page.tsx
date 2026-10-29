// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link, useParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { BookInfo, BooksApi, BookSummary, TocLine, TocNode } from "./api";
import { MarkdownReader } from "./markdown";

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

const isNode = (line: TocLine): line is TocNode => !("kind" in line);

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

function Section({
  api,
  book,
  section,
}: {
  readonly api: BooksApi;
  readonly book: string;
  readonly section: string;
}) {
  const loaded = useLoaded(() => api.section(book, section), [api, book, section], failureText);
  return (
    <Ready loaded={loaded}>
      {(found) => (
        <article className="vl-book-section">
          <h2>{found.number == undefined ? found.title : `${found.number} ${found.title}`}</h2>
          <MarkdownReader markdown={found.markdown} title={found.title} />
        </article>
      )}
    </Ready>
  );
}

function Book({
  api,
  info,
  section,
}: {
  readonly api: BooksApi;
  readonly info: BookInfo;
  readonly section: string | undefined;
}) {
  const t = useT();
  const shown = section ?? firstSection(info.toc);
  return (
    <>
      <p className="vl-muted">
        <Link to="/books">{t("books")}</Link>
      </p>
      <h1>{info.title}</h1>
      <div className="vl-book">
        <nav className="vl-book-toc" aria-label={t("books-contents")}>
          <TocList book={info.id} lines={info.toc} current={shown} />
        </nav>
        {shown === undefined ? (
          <p className="vl-muted">{t("books-no-sections")}</p>
        ) : (
          <Section api={api} book={info.id} section={shown} />
        )}
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
    <Ready loaded={loaded}>{(info) => <Book api={api} info={info} section={section} />}</Ready>
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
