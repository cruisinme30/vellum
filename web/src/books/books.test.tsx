// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { ServerApisProvider } from "../data/server-apis";
import { createMemoryStore } from "../data/store";
import { WorkspaceProvider } from "../data/workspace-context";
import type { CollabConnector } from "../editor/collab/provider";
import { I18nProvider } from "../i18n/i18n";

import { createHttpBooksApi, type BookInfo, type BooksApi } from "./api";
import { BookPage, BooksPage, firstSection } from "./books-page";
import { MarkdownReader } from "./markdown";

const node = (id: string, title: string, extra: object = {}) => ({
  ...(id === "" ? {} : { id }),
  title,
  label: title,
  depth: 1,
  draft: false,
  children: [],
  ...extra,
});

const BOOK: BookInfo = {
  id: "b1",
  title: "Handbook",
  max_depth: 3,
  numbering: "decimal",
  role: "reader",
  toc: [
    { kind: "part", title: "Start" },
    node("", "Draft", { draft: true }),
    node("s1", "Welcome", {
      number: "1",
      children: [node("s2", "Setup", { number: "1.1" })],
    }),
    { kind: "separator" },
  ],
};

/** Only the reading operations; a reader never reaches the others. */
function fakeApi(): BooksApi {
  const reading: Pick<BooksApi, "list" | "book" | "section"> = {
    list: vi.fn(() =>
      Promise.resolve([{ id: "b1", title: "Handbook", path: "handbook.book" }]),
    ),
    book: vi.fn(() => Promise.resolve(BOOK)),
    section: vi.fn((_book: string, section: string) =>
      Promise.resolve({
        id: section,
        book_id: "b1",
        title: section === "s1" ? "Welcome" : "Setup",
        label: "x",
        number: section === "s1" ? "1" : "1.1",
        path: "x.md",
        restricted: false,
        source: "x",
        markdown:
          section === "s1"
            ? "# Welcome\n\nHello **team**."
            : "Run `vellum serve`.",
      }),
    ),
  };
  return reading as BooksApi;
}

/** Priya's drives, with the folders books can be made in. */
const workspace = createMemoryStore({
  name: "Northstar",
  me: { handle: "priya", name: "Priya Raman" },
  people: [],
  spaces: [
    {
      id: "S1",
      slug: "priya",
      name: "My Drive",
      icon: "🏠",
      root: "drives/users/priya",
    },
    {
      id: "S2",
      slug: "engineering",
      name: "Engineering",
      icon: "👥",
      root: "drives/shared/engineering",
    },
  ],
  pages: [],
  bodies: {},
});

function renderAt(path: string, api?: BooksApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <WorkspaceProvider store={workspace} collab={{} as CollabConnector}>
        <ServerApisProvider apis={{ books: api }}>
          <MemoryRouter initialEntries={[path]}>
            <Routes>
              <Route path="books" element={<BooksPage />} />
              <Route path="books/:bookId" element={<BookPage />} />
              <Route path="books/:bookId/:sectionId" element={<BookPage />} />
            </Routes>
          </MemoryRouter>
        </ServerApisProvider>
      </WorkspaceProvider>
    </I18nProvider>,
  );
  return user;
}

describe("books API", () => {
  it("calls the book routes", async () => {
    const get = vi.fn(() => Promise.resolve({ books: [] }));
    const send = vi.fn(() => Promise.resolve({}));
    const api = createHttpBooksApi({
      get,
      send,
      forgetSession: vi.fn(),
    } as unknown as Http);
    await api.list();
    await api.section("b 1", "s/2");
    expect(get).toHaveBeenLastCalledWith("/api/v1/books/b%201/sections/s%2F2");
    await api.create("Field guide", "drives/users/priya");
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/books", {
      title: "Field guide",
      folder: "drives/users/priya",
    });
  });
});

describe("books", () => {
  it("lists books and reads one section by section", async () => {
    const api = fakeApi();
    const user = renderAt("/books", api);
    await user.click(await screen.findByRole("link", { name: "Handbook" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "Handbook" }),
    ).toBeDefined();
    const article = await screen.findByRole("article");
    expect(within(article).getByRole("heading", { level: 2 }).textContent).toBe(
      "1 Welcome",
    );
    // The section's own title heading is not repeated.
    expect(within(article).getAllByRole("heading")).toHaveLength(1);
    expect(within(article).getByText("team").tagName).toBe("STRONG");

    const contents = screen.getByRole("navigation", { name: "Contents" });
    expect(within(contents).getByText("Draft").tagName).toBe("SPAN");
    await user.click(within(contents).getByRole("link", { name: "1.1 Setup" }));
    const code = await screen.findByText("vellum serve");
    expect(code.tagName).toBe("CODE");
    expect(
      within(contents)
        .getByRole("link", { name: "1.1 Setup" })
        .getAttribute("aria-current"),
    ).toBe("page");
  });

  it("makes a book in the chosen drive and opens it (#566)", async () => {
    const api = fakeApi();
    const create = vi.fn(() =>
      Promise.resolve({
        id: "b1",
        title: "Handbook",
        path: "drives/shared/engineering/Handbook.book",
      }),
    );
    const user = renderAt("/books", { ...api, create });
    await user.click(await screen.findByRole("button", { name: "New book" }));
    const form = screen.getByRole("form", { name: "New book" });
    const create_ = within(form).getByRole("button", { name: "Create book" });
    expect(create_).toHaveProperty("disabled", true);
    await user.type(
      within(form).getByRole("textbox", { name: "Title" }),
      "Handbook",
    );
    expect(
      within(form).getByRole("combobox", { name: "Drive" }),
    ).toHaveProperty("value", "drives/users/priya");
    await user.selectOptions(
      within(form).getByRole("combobox", { name: "Drive" }),
      "Engineering",
    );
    await user.click(create_);
    expect(create).toHaveBeenCalledWith(
      "Handbook",
      "drives/shared/engineering",
    );
    expect(
      await screen.findByRole("heading", { level: 1, name: "Handbook" }),
    ).toBeDefined();
  });

  it("opens the New book form from New → Book, and says why a create failed", async () => {
    const create = vi.fn(() =>
      Promise.reject(new Error("You cannot create books in this drive.")),
    );
    const user = renderAt("/books?new=book", { ...fakeApi(), create });
    const form = await screen.findByRole("form", { name: "New book" });
    await user.type(
      within(form).getByRole("textbox", { name: "Title" }),
      "Notes",
    );
    await user.click(within(form).getByRole("button", { name: "Create book" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(
      "You cannot create books in this drive.",
    );
    await user.click(within(form).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("form", { name: "New book" })).toBeNull();
    expect(screen.getByRole("button", { name: "New book" })).toBeDefined();
  });

  it("finds the first readable section", () => {
    expect(firstSection(BOOK.toc)).toBe("s1");
    expect(
      firstSection([node("", "Empty", { children: [node("deep", "Deep")] })]),
    ).toBe("deep");
    expect(firstSection([])).toBeUndefined();
  });

  it("explains itself in demo mode", () => {
    renderAt("/books");
    expect(screen.getByText(/needs a Vellum server/)).toBeDefined();
  });
});

describe("markdown reader", () => {
  it("reads blocks and inline marks without running markup", () => {
    const { container } = render(
      <MarkdownReader
        markdown={[
          "## Steps ##",
          "",
          "1. One",
          "2. *Two*",
          "",
          "- a",
          "- [docs](https://vellum.dev)",
          "- [bad](javascript:alert(1))",
          "",
          "> quoted",
          "",
          "---",
          "",
          "```",
          "<script>x</script>",
          "```",
          "",
          "<b>raw</b> text",
        ].join("\n")}
      />,
    );
    expect(container.querySelector("h3")?.textContent).toBe("Steps");
    expect(container.querySelectorAll("ol li")).toHaveLength(2);
    expect(container.querySelector("ol em")?.textContent).toBe("Two");
    expect(container.querySelector("a")?.getAttribute("href")).toBe(
      "https://vellum.dev",
    );
    expect(container.querySelectorAll("a")).toHaveLength(1);
    expect(container.querySelector("blockquote")?.textContent).toBe("quoted");
    expect(container.querySelector("hr")).not.toBeNull();
    expect(container.querySelector("pre code")?.textContent).toBe(
      "<script>x</script>",
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("b")).toBeNull();
  });
});
