// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Books over the server (`/api/v1/books`): reading, changing the outline,
 * section access, history, snapshots and publishing.
 */

import type { Operations } from "../api/api.gen";
import type { Http } from "../api/http";

type Out<K extends keyof Operations> = Operations[K]["output"];
type In<K extends keyof Operations> = Operations[K]["input"];

export type BookSummary = Out<"books.list">["books"][number];
export type BookInfo = Out<"books.get">;
export type TocLine = BookInfo["toc"][number];
export type TocNode = Extract<TocLine, { readonly children: unknown }>;
export type SectionDocument = Out<"books.get_section">;
type SectionAccess = Out<"books.set_section_access">;
/** Where a section goes: relative to a section, or at either end. */
export type Place = In<"books.move_section">["place"];
type SnapshotInfo = Out<"books.create_snapshot">;
export type BookDiff = Out<"books.diff">;
export type HistoryEntry = Out<"books.history">["entries"][number];
type Created = Out<"books.copy_section">;
export type Publication = Out<"books.publish">;
export type PublishFormat = Publication["format"];

/** A section's own access settings, as `books.set_section_access` takes them. */
interface AccessInput {
  readonly restrictTo: readonly string[];
  readonly owners: readonly string[];
  readonly reviewers: readonly string[];
}

/** The book operations the web app uses. */
export interface BooksApi {
  list(): Promise<BookSummary[]>;
  /** Makes an empty book in a drive folder, such as `drives/users/priya`. */
  create(title: string, folder: string): Promise<BookSummary>;
  /** A book with its table of contents. */
  book(id: string): Promise<BookInfo>;
  section(book: string, section: string): Promise<SectionDocument>;
  addSection(book: string, title: string, place: Place): Promise<SectionDocument>;
  moveSection(book: string, section: string, place: Place): Promise<SectionDocument>;
  renameSection(book: string, section: string, title: string): Promise<SectionDocument>;
  removeSection(book: string, section: string): Promise<readonly string[]>;
  copySection(book: string, section: string, place: Place): Promise<Created>;
  /** Moves (or copies) a section with everything under it to the end of another book. */
  transferSection(book: string, section: string, to: string, copy: boolean): Promise<Created>;
  splitSection(book: string, section: string): Promise<Created>;
  mergeSection(book: string, section: string): Promise<readonly string[]>;
  setAccess(book: string, section: string, access: AccessInput): Promise<SectionAccess>;
  history(book: string): Promise<HistoryEntry[]>;
  restoreOutline(book: string, entry: string): Promise<BookInfo>;
  snapshots(book: string): Promise<SnapshotInfo[]>;
  createSnapshot(book: string, tag: string, message: string): Promise<SnapshotInfo>;
  /** What changed from snapshot `from` to snapshot `to`, or to now. */
  diff(book: string, from: string, to?: string): Promise<BookDiff>;
  publish(book: string, format: PublishFormat, audience: readonly string[]): Promise<Publication>;
  publications(book: string): Promise<Publication[]>;
}

const BOOKS = "/api/v1/books";
const segment = encodeURIComponent;

const bookPath = (book: string) => `${BOOKS}/${segment(book)}`;
const sectionPath = (book: string, section: string) =>
  `${bookPath(book)}/sections/${segment(section)}`;

/** Books over a Vellum server. */
export function createHttpBooksApi(http: Http): BooksApi {
  return {
    async list() {
      const found = await http.get<Out<"books.list">>(BOOKS);
      return found.books.toSorted((a, b) => a.title.localeCompare(b.title));
    },
    create: (title, folder) =>
      http.send<Out<"books.create">>("POST", BOOKS, { title, folder } satisfies In<"books.create">),
    book: (id) => http.get<BookInfo>(bookPath(id)),
    section: (book, section) => http.get<SectionDocument>(sectionPath(book, section)),
    addSection: (book, title, place) =>
      http.send<SectionDocument>("POST", `${bookPath(book)}/sections`, { title, place }),
    moveSection: (book, section, place) =>
      http.send<SectionDocument>("POST", `${sectionPath(book, section)}/move`, { place }),
    renameSection: (book, section, title) =>
      http.send<SectionDocument>("PATCH", sectionPath(book, section), { title }),
    async removeSection(book, section) {
      const out = await http.send<Out<"books.remove_section">>(
        "DELETE",
        sectionPath(book, section),
      );
      return out.removed;
    },
    copySection: (book, section, place) =>
      http.send<Created>("POST", `${sectionPath(book, section)}/copy`, { place }),
    transferSection: (book, section, to, copy) =>
      http.send<Created>("POST", `${sectionPath(book, section)}/transfer`, {
        to_book: to,
        place: { at: "end" },
        copy,
      }),
    splitSection: (book, section) =>
      http.send<Created>("POST", `${sectionPath(book, section)}/split`, {}),
    async mergeSection(book, section) {
      const out = await http.send<Out<"books.merge_section">>(
        "POST",
        `${sectionPath(book, section)}/merge`,
        {},
      );
      return out.removed;
    },
    setAccess: (book, section, access) =>
      http.send<SectionAccess>("PUT", `${sectionPath(book, section)}/access`, {
        restrict_to: access.restrictTo,
        owners: access.owners,
        reviewers: access.reviewers,
      }),
    async history(book) {
      const out = await http.get<Out<"books.history">>(`${bookPath(book)}/history`);
      return out.entries;
    },
    restoreOutline: (book, entry) =>
      http.send<BookInfo>("POST", `${bookPath(book)}/history/restore`, { entry }),
    async snapshots(book) {
      const out = await http.get<Out<"books.list_snapshots">>(`${bookPath(book)}/snapshots`);
      return out.snapshots.toSorted((a, b) => b.created_at.localeCompare(a.created_at));
    },
    createSnapshot: (book, tag, message) =>
      http.send<SnapshotInfo>("POST", `${bookPath(book)}/snapshots`, { tag, message }),
    diff: (book, from, to) => http.get<BookDiff>(`${bookPath(book)}/diff`, { from, to }),
    publish: (book, format, audience) =>
      http.send<Publication>("POST", `${bookPath(book)}/publish`, { format, audience }),
    async publications(book) {
      const out = await http.get<Out<"books.publications">>(`${bookPath(book)}/publications`);
      return out.publications;
    },
  };
}
