// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi, type Mock } from "vitest";

import type { Http } from "../api/http";
import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import { createHttpBooksApi, type BookInfo, type BooksApi, type SectionDocument } from "./api";
import { BookPage } from "./books-page";
import { dropPlace, isWithin, outlineRows, placeFor, principalList } from "./outline";

const node = (id: string, title: string, extra: object = {}) => ({
  id,
  title,
  label: title,
  depth: 1,
  draft: false,
  children: [],
  ...extra,
});

const book = (role: string): BookInfo => ({
  id: "b1",
  title: "Handbook",
  max_depth: 3,
  numbering: "decimal",
  role,
  toc: [
    node("s1", "Welcome", {
      number: "1",
      label: "§1 Welcome",
      children: [node("s2", "Setup", { number: "1.1", depth: 2, label: "§1.1 Setup" })],
    }),
    node("s3", "Glossary", { number: "2", label: "§2 Glossary" }),
  ],
});

const sectionDocument = (id: string, manager: boolean): SectionDocument => ({
  id,
  book_id: "b1",
  title: id === "s1" ? "Welcome" : "Glossary",
  label: "x",
  number: "1",
  path: "x.md",
  restricted: false,
  source: "x",
  markdown: "Hello.",
  ...(manager ? { access: { restricted_by: [], owners: [], reviewers: [] } } : {}),
});

const created = { book_id: "b1", section_id: "s9", created: ["s9"] };
const publication = {
  format: "site" as const,
  public: true,
  at: "2026-10-29T21:00:00Z",
  by: "user:1",
  folder: "Handbook.book/.published/site",
  files: 4,
  url: "/published/books/b1/",
};

/** The API with every operation a mock, so tests can check the calls. */
type FakeApi = { readonly [K in keyof BooksApi]: Mock<BooksApi[K]> };

function fakeApi(role: string): FakeApi {
  const manager = role === "manager" || role === "owner";
  return {
    list: vi.fn(() =>
      Promise.resolve([
        { id: "b1", title: "Handbook", path: "h.book" },
        { id: "b2", title: "Guide", path: "g.book" },
      ]),
    ),
    book: vi.fn(() => Promise.resolve(book(role))),
    section: vi.fn((_b: string, s: string) => Promise.resolve(sectionDocument(s, manager))),
    addSection: vi.fn(() => Promise.resolve(sectionDocument("s9", manager))),
    moveSection: vi.fn(() => Promise.resolve(sectionDocument("s3", manager))),
    renameSection: vi.fn(() => Promise.resolve(sectionDocument("s1", manager))),
    removeSection: vi.fn(() => Promise.resolve(["s1"])),
    copySection: vi.fn(() => Promise.resolve(created)),
    transferSection: vi.fn(() => Promise.resolve({ ...created, book_id: "b2" })),
    splitSection: vi.fn(() => Promise.resolve(created)),
    mergeSection: vi.fn(() => Promise.resolve(["s2"])),
    setAccess: vi.fn(() =>
      Promise.resolve({
        restricted_by: ["s1"],
        owners: [],
        reviewers: [],
        own_restrict_to: ["group:1"],
        restrict_to: ["group:1"],
      }),
    ),
    history: vi.fn(() =>
      Promise.resolve([
        {
          id: "h1",
          at: "2026-10-29T20:00:00Z",
          by: "user:1",
          op: "move_section",
          outline: [
            { id: "s3", title: "Glossary", label: "§1 Glossary", depth: 1, region: "numbered" },
            { id: "s1", title: "Welcome", label: "§2 Welcome", depth: 1, region: "numbered" },
          ],
        },
      ]),
    ),
    restoreOutline: vi.fn(() => Promise.resolve(book(role))),
    snapshots: vi.fn(() =>
      Promise.resolve([
        { tag: "v1.0", message: "First", created_at: "2026-10-29T19:00:00Z", created_by: "" },
      ]),
    ),
    createSnapshot: vi.fn(() =>
      Promise.resolve({ tag: "v1.1", message: "", created_at: "", created_by: "" }),
    ),
    diff: vi.fn(() =>
      Promise.resolve({
        added: [{ id: "s9", label: "§3 New" }],
        removed: [],
        moved: [{ id: "s3", from: "§1 Glossary", to: "§2 Glossary" }],
        changed: [],
        settings: [],
        summary_patch: "",
      }),
    ),
    publish: vi.fn(() => Promise.resolve(publication)),
    publications: vi.fn(() => Promise.resolve([publication])),
  };
}

function renderBook(api: FakeApi, path = "/books/b1/s1") {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ books: api }}>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="books/:bookId" element={<BookPage />} />
            <Route path="books/:bookId/:sectionId" element={<BookPage />} />
          </Routes>
        </MemoryRouter>
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

describe("outline", () => {
  const rows = outlineRows(book("editor").toc);

  it("lists sections with their place", () => {
    expect(rows.map((r) => [r.id, r.level, r.parent ?? "-", r.height])).toEqual([
      ["s1", 1, "-", 2],
      ["s2", 2, "s1", 1],
      ["s3", 1, "-", 1],
    ]);
  });

  it("turns key presses into places", () => {
    expect(placeFor(rows, "s3", "up", 3)).toEqual({ at: "before", section: "s1" });
    expect(placeFor(rows, "s1", "up", 3)).toBeUndefined();
    expect(placeFor(rows, "s3", "indent", 3)).toEqual({ at: "last_child_of", section: "s1" });
    expect(placeFor(rows, "s3", "indent", 1)).toBeUndefined();
    expect(placeFor(rows, "s2", "outdent", 3)).toEqual({ at: "after", section: "s1" });
    expect(placeFor(rows, "s1", "down", 3)).toEqual({ at: "after", section: "s3" });
  });

  it("turns drops into places and never nests a section in itself", () => {
    const [s1, s2, s3] = rows;
    if (!s1 || !s2 || !s3) throw new Error("rows");
    expect(dropPlace(s1, s3, 1, 40, 3)).toEqual({ at: "before", section: "s1" });
    expect(dropPlace(s1, s3, 39, 40, 3)).toEqual({ at: "after", section: "s1" });
    expect(dropPlace(s1, s3, 20, 40, 3)).toEqual({ at: "last_child_of", section: "s1" });
    expect(dropPlace(s2, s1, 20, 40, 2)).toBeUndefined();
    expect(isWithin(rows, "s2", "s1")).toBe(true);
    expect(isWithin(rows, "s3", "s1")).toBe(false);
  });

  it("reads principal lists", () => {
    expect(principalList("user:A, group:B\n\n user:C ")).toEqual(["user:A", "group:B", "user:C"]);
  });
});

describe("books API writes", () => {
  it("calls the editing routes", async () => {
    const send = vi.fn(() => Promise.resolve({ removed: [], entries: [], publications: [] }));
    const get = vi.fn(() => Promise.resolve({ entries: [], publications: [] }));
    const api = createHttpBooksApi({ get, send, forgetSession: vi.fn() } as unknown as Http);
    await api.moveSection("b1", "s 1", { at: "before", section: "s2" });
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/books/b1/sections/s%201/move", {
      place: { at: "before", section: "s2" },
    });
    await api.transferSection("b1", "s1", "b2", true);
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/books/b1/sections/s1/transfer", {
      to_book: "b2",
      place: { at: "end" },
      copy: true,
    });
    await api.setAccess("b1", "s1", { restrictTo: ["group:1"], owners: [], reviewers: [] });
    expect(send).toHaveBeenLastCalledWith("PUT", "/api/v1/books/b1/sections/s1/access", {
      restrict_to: ["group:1"],
      owners: [],
      reviewers: [],
    });
    await api.publish("b1", "mdbook", []);
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/books/b1/publish", {
      format: "mdbook",
      audience: [],
    });
    await api.history("b1");
    expect(get).toHaveBeenLastCalledWith("/api/v1/books/b1/history");
    await api.diff("b1", "v1");
    expect(get).toHaveBeenLastCalledWith("/api/v1/books/b1/diff", { from: "v1", to: undefined });
  });
});

describe("editing a book", () => {
  it("readers get the plain contents and no tools", async () => {
    renderBook(fakeApi("viewer"));
    await screen.findByRole("article");
    expect(screen.queryByRole("tree")).toBeNull();
    expect(screen.queryByRole("toolbar", { name: "Section tools" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Publish" })).toBeNull();
  });

  it("reorders and nests from the keyboard", async () => {
    const api = fakeApi("editor");
    const user = renderBook(api);
    const tree = await screen.findByRole("tree", { name: "Contents" });
    const glossary = within(tree).getByRole("treeitem", { name: "2 Glossary" });
    expect(glossary.getAttribute("aria-level")).toBe("1");
    glossary.focus();
    await user.keyboard("{Alt>}{Shift>}{ArrowUp}{/Shift}{/Alt}");
    expect(api.moveSection).toHaveBeenLastCalledWith("b1", "s3", {
      at: "before",
      section: "s1",
    });
    await user.keyboard("{Alt>}{Shift>}{ArrowRight}{/Shift}{/Alt}");
    expect(api.moveSection).toHaveBeenLastCalledWith("b1", "s3", {
      at: "last_child_of",
      section: "s1",
    });
    expect(await screen.findByText("Moved Glossary.")).toBeDefined();
  });

  it("nests by dragging onto a section", async () => {
    const api = fakeApi("editor");
    renderBook(api);
    const tree = await screen.findByRole("tree", { name: "Contents" });
    const glossary = within(tree).getByRole("treeitem", { name: "2 Glossary" });
    const welcome = within(tree).getByRole("treeitem", { name: "1 Welcome" });
    fireEvent.dragStart(glossary, { dataTransfer: { setData: vi.fn() } });
    fireEvent.dragOver(welcome, { dataTransfer: {}, clientY: 0 });
    fireEvent.drop(welcome, { dataTransfer: {}, clientY: 0 });
    expect(api.moveSection).toHaveBeenLastCalledWith("b1", "s3", {
      at: "last_child_of",
      section: "s1",
    });
  });

  it("adds, moves between books, splits and folds sections", async () => {
    const api = fakeApi("editor");
    const user = renderBook(api);
    await user.click(await screen.findByRole("button", { name: "Headings to sections" }));
    expect(api.splitSection).toHaveBeenCalledWith("b1", "s1");
    await user.click(await screen.findByRole("button", { name: "Sections to headings" }));
    expect(api.mergeSection).toHaveBeenCalledWith("b1", "s1");
    const tools = await screen.findByRole("toolbar", { name: "Section tools" });
    await user.click(within(tools).getByRole("button", { name: "Add section after" }));
    await user.type(screen.getByLabelText("Section title"), "Billing");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(api.addSection).toHaveBeenCalledWith("b1", "Billing", { at: "after", section: "s1" });

    await user.click(await screen.findByRole("button", { name: "Move to another book" }));
    const to = await screen.findByLabelText("Book");
    expect(
      within(to)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["Guide"]);
    await user.click(screen.getByLabelText("Copy it (new section IDs)"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    // Adding opened the new section, so that is the one moved.
    expect(api.transferSection).toHaveBeenCalledWith("b1", "s9", "b2", true);
  });

  it("shows the outline history and restores an order", async () => {
    const api = fakeApi("editor");
    const user = renderBook(api);
    await user.click(await screen.findByRole("button", { name: "Outline history" }));
    const entry = await screen.findByRole("button", { name: /Section moved/ });
    await user.click(entry);
    expect(screen.getByText("now §2 Glossary")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Restore this order" }));
    expect(api.restoreOutline).toHaveBeenCalledWith("b1", "h1");
  });

  it("takes snapshots and compares them", async () => {
    const api = fakeApi("editor");
    const user = renderBook(api);
    await user.click(await screen.findByRole("button", { name: "Snapshots" }));
    await user.type(await screen.findByLabelText("Tag"), "v1.1");
    await user.click(screen.getByRole("button", { name: "Take snapshot" }));
    expect(api.createSnapshot).toHaveBeenCalledWith("b1", "v1.1", "");
    await user.click(await screen.findByRole("button", { name: "Compare" }));
    expect(api.diff).toHaveBeenCalledWith("b1", "v1.0", undefined);
    expect(await screen.findByText("§1 Glossary → §2 Glossary")).toBeDefined();
  });

  it("lets managers restrict sections and publish", async () => {
    const api = fakeApi("manager");
    const user = renderBook(api);
    await user.click(await screen.findByRole("button", { name: "Section access" }));
    await user.type(await screen.findByLabelText(/Only these can see it/), "group:1");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(api.setAccess).toHaveBeenCalledWith("b1", "s1", {
      restrictTo: ["group:1"],
      owners: [],
      reviewers: [],
    });

    await user.click(screen.getByRole("button", { name: "Publish" }));
    const panel = await screen.findByRole("region", { name: "Publish" });
    expect(
      await within(panel).findByRole("link", { name: "Open the published site" }),
    ).toBeDefined();
    await user.click(within(panel).getByLabelText("mdBook source"));
    await user.click(within(panel).getByRole("button", { name: "Publish" }));
    expect(api.publish).toHaveBeenCalledWith("b1", "mdbook", []);
  });
});
