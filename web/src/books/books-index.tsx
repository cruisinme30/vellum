// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import type { BooksApi, BookSummary } from "./api";

/** Where a book opens. */
export const bookPath = (book: string) => `/books/${encodeURIComponent(book)}`;

/**
 * Makes an empty book (#566, D323): a title, and which of the person's
 * drives it goes in, My Drive first.
 */
function NewBookForm({
  api,
  onCreated,
  onCancel,
}: {
  readonly api: BooksApi;
  readonly onCreated: (book: BookSummary) => void;
  readonly onCancel: () => void;
}) {
  const t = useT();
  const drives = useWorkspace().spaces.filter((space) => space.root !== undefined);
  const [title, setTitle] = useState("");
  const [folder, setFolder] = useState(drives[0]?.root ?? "");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  return (
    <form
      className="vl-book-new"
      aria-label={t("books-new")}
      onSubmit={(event) => {
        event.preventDefault();
        setBusy(true);
        setError(undefined);
        api.create(title.trim(), folder).then(onCreated, (error_: unknown) => {
          setError(failureText(error_));
          setBusy(false);
        });
      }}
    >
      <label className="vl-field">
        {t("books-new-book-title")}
        <input
          required
          maxLength={200}
          value={title}
          // The form opens on purpose, from New → Book or the button above it.
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
          onChange={(event) => {
            setTitle(event.target.value);
          }}
        />
      </label>
      <label className="vl-field">
        {t("books-new-where")}
        <select
          required
          value={folder}
          onChange={(event) => {
            setFolder(event.target.value);
          }}
        >
          {drives.map((space) => (
            <option key={space.id} value={space.root}>
              {space.name}
            </option>
          ))}
        </select>
      </label>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <div className="vl-book-new-actions">
        <button type="button" className="vl-text-button" onClick={onCancel}>
          {t("books-cancel")}
        </button>
        <button type="submit" className="vl-filled-button" disabled={busy || title.trim() === ""}>
          {t("books-create")}
        </button>
      </div>
    </form>
  );
}

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

export function AllBooks({ api }: { readonly api: BooksApi }) {
  const t = useT();
  const navigate = useNavigate();
  const [query, setQuery] = useSearchParams();
  const creating = query.get("new") === "book";
  const setCreating = (open: boolean) => {
    setQuery(open ? { new: "book" } : {}, { replace: true });
  };
  const loaded = useLoaded(() => api.list(), [api], failureText);
  return (
    <>
      {creating ? (
        <NewBookForm
          api={api}
          onCreated={(book) => {
            void navigate(bookPath(book.id));
          }}
          onCancel={() => {
            setCreating(false);
          }}
        />
      ) : (
        <button
          type="button"
          className="vl-filled-button vl-book-new-open"
          onClick={() => {
            setCreating(true);
          }}
        >
          {t("books-new")}
        </button>
      )}
      <Ready loaded={loaded}>{(books) => <BookList books={books} />}</Ready>
    </>
  );
}
