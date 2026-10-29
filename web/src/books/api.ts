// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Reading books (`/api/v1/books`): the list, a book's contents and a section. */

import type {
  BookInfo,
  BookList,
  BookSummary,
  SectionDoc as SectionDocument,
} from "../api/api.gen";
import type { Http } from "../api/http";

/** The book operations the web app uses. */
export interface BooksApi {
  list(): Promise<BookSummary[]>;
  /** A book with its table of contents. */
  book(id: string): Promise<BookInfo>;
  section(book: string, section: string): Promise<SectionDocument>;
}

const BOOKS = "/api/v1/books";
const segment = encodeURIComponent;

/** Books over a Vellum server. */
export function createHttpBooksApi(http: Http): BooksApi {
  return {
    async list() {
      const found = await http.get<BookList>(BOOKS);
      return found.books.toSorted((a, b) => a.title.localeCompare(b.title));
    },
    book: (id) => http.get<BookInfo>(`${BOOKS}/${segment(id)}`),
    section: (book, section) =>
      http.get<SectionDocument>(`${BOOKS}/${segment(book)}/sections/${segment(section)}`),
  };
}

export type { BookInfo, BookSummary, TocLine, TocNode } from "../api/api.gen";
