// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  act,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";

async function renderBoard() {
  const collections = createMockCollectionsStore();
  const rendered = renderApp(
    `/data/${COLLECTION.issues}?view=Board`,
    undefined,
    collections,
  );
  await screen.findByRole("region", { name: /^Backlog/ });
  const column = (name: string) =>
    screen.getByRole("region", { name: new RegExp(String.raw`^${name} \d`) });
  const card = (title: string) =>
    screen.getByRole("button", { name: new RegExp(`^${title}`) });
  const titlesIn = (name: string) =>
    within(column(name))
      .queryAllByRole("button", { name: /^(?!Move )/ })
      .map((c) => c.querySelector(".vl-card-title")?.textContent);
  return { ...rendered, column, card, titlesIn };
}

function dataTransfer(types: string[] = []) {
  const data = new Map<string, string>();
  return {
    effectAllowed: "none",
    types,
    setData: (type: string, value: string) => {
      data.set(type, value);
      types.push(type);
    },
    getData: (type: string) => data.get(type) ?? "",
  };
}

describe("BoardView", () => {
  it("shows a column per status, with counts and cards", async () => {
    const { column, titlesIn } = await renderBoard();
    for (const [name, count] of [
      ["Backlog", "3"],
      ["In progress", "2"],
      ["In review", "1"],
      ["Done", "2"],
    ] as const) {
      expect(
        within(column(name)).getByRole("heading", { level: 2 }).textContent,
      ).toBe(`${name} ${count}`);
    }
    expect(titlesIn("In review")).toEqual([
      "Gateway returns 502 on cold start",
    ]);
    expect(screen.getByText(/Drag a card to another column/)).toBeDefined();
    // The grouping property is not repeated on the cards.
    expect(within(column("Done")).queryByText("Status")).toBeNull();
    expect(within(column("Done")).getAllByText("Owner")).toHaveLength(2);
  });

  it("moves a card with Alt+Right and Alt+Left, keeping focus on it", async () => {
    const { user, card, titlesIn } = await renderBoard();
    act(() => {
      card("Gateway returns 502").focus();
    });
    await user.keyboard("{Alt>}{ArrowRight}{/Alt}");
    await waitFor(() => {
      expect(titlesIn("Done")).toContain("Gateway returns 502 on cold start");
    });
    expect(document.activeElement).toBe(card("Gateway returns 502"));
    expect(screen.getByRole("status").textContent).toBe(
      "Moved Gateway returns 502 on cold start to Done.",
    );
    await user.keyboard("{Alt>}{ArrowRight}{/Alt}");
    await user.keyboard("{Alt>}{ArrowLeft}{/Alt}");
    await waitFor(() => {
      expect(titlesIn("In review")).toContain(
        "Gateway returns 502 on cold start",
      );
    });
    act(() => {
      card("Token bucket").focus();
    });
    await user.keyboard("{ArrowRight}");
    expect(titlesIn("In progress")).toContain("Token bucket per tenant");
  });

  it("moves a card from its Move to menu", async () => {
    const { user, column, titlesIn } = await renderBoard();
    const move = within(column("In review")).getByRole("button", {
      name: "Move Gateway returns 502 on cold start",
    });
    await user.click(move);
    const menu = screen.getByRole("menu");
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((index) => index.textContent),
    ).toEqual(["Move to Backlog", "Move to In progress", "Move to Done"]);
    expect(document.activeElement).toBe(
      within(menu).getAllByRole("menuitem")[0],
    );
    await user.keyboard("{ArrowUp}{ArrowDown}{End}{Home}{ArrowDown}{Enter}");
    await waitFor(() => {
      expect(titlesIn("In progress")).toContain(
        "Gateway returns 502 on cold start",
      );
    });
    const again = screen.getByRole("button", {
      name: "Move Gateway returns 502 on cold start",
    });
    await user.click(again);
    await user.click(screen.getByRole("menuitem", { name: "Move to Done" }));
    await waitFor(() => {
      expect(titlesIn("Done")).toContain("Gateway returns 502 on cold start");
    });
  });

  it("closes the menu with Escape and Space picks", async () => {
    const { user, column } = await renderBoard();
    const move = within(column("Done")).getAllByRole("button", {
      name: /^Move /,
    })[0];
    if (!move) {
      throw new Error("no move button");
    }
    await user.click(move);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(move);
    await user.click(move);
    await user.click(screen.getByRole("menu"));
    await user.keyboard("{Tab}");
    expect(screen.queryByRole("menu")).toBeNull();
    await user.click(move);
    await user.keyboard(" ");
    await waitFor(() => {
      expect(
        within(column("Backlog")).getAllByRole("button", { name: /^Move / }),
      ).toHaveLength(4);
    });
  });

  it("moves a card by dragging it to another column", async () => {
    const { card, column, titlesIn } = await renderBoard();
    const transfer = dataTransfer();
    fireEvent.dragStart(card("Token bucket"), { dataTransfer: transfer });
    expect(transfer.effectAllowed).toBe("move");
    fireEvent.dragOver(column("In review"), { dataTransfer: transfer });
    expect(column("In review").dataset["over"]).toBe("true");
    fireEvent.dragLeave(column("In review"), { dataTransfer: transfer });
    expect(column("In review").dataset["over"]).toBeUndefined();
    fireEvent.drop(column("In review"), { dataTransfer: transfer });
    await waitFor(() => {
      expect(titlesIn("In review")).toContain("Token bucket per tenant");
    });
  });

  it("ignores drops of other things and onto the card's own column", async () => {
    const { card, column, collections } = await renderBoard();
    const spy = vi.spyOn(collections, "updateRow");
    fireEvent.dragOver(column("Done"), {
      dataTransfer: dataTransfer(["text/plain"]),
    });
    expect(column("Done").dataset["over"]).toBeUndefined();
    fireEvent.drop(column("Done"), { dataTransfer: dataTransfer() });
    const transfer = dataTransfer();
    fireEvent.dragStart(card("Token bucket"), { dataTransfer: transfer });
    fireEvent.drop(column("In progress"), { dataTransfer: transfer });
    expect(spy).not.toHaveBeenCalled();
  });

  it("opens a card's row with a click, Enter or Space", async () => {
    const { user, card } = await renderBoard();
    await user.click(card("Token bucket"));
    expect(
      await screen.findByRole("dialog", { name: "Token bucket per tenant" }),
    ).toBeDefined();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(card("Token bucket"));
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("dialog")).toBeDefined();
    await user.keyboard("{Escape}");
    await user.keyboard(" ");
    expect(await screen.findByRole("dialog")).toBeDefined();
  });

  it("says when a move is refused", async () => {
    const { user, card, collections } = await renderBoard();
    vi.spyOn(collections, "updateRow").mockRejectedValueOnce(
      new Error("no write access"),
    );
    act(() => {
      card("Token bucket").focus();
    });
    await user.keyboard("{Alt>}{ArrowLeft}{/Alt}");
    await waitFor(() => {
      expect(screen.getByRole("status").textContent).toBe("no write access");
    });
  });

  it("has no move controls when the grouping is computed", async () => {
    const collections = createMockCollectionsStore();
    const info = collections.info.bind(collections);
    const query = collections.query.bind(collections);
    vi.spyOn(collections, "info").mockImplementation(async (id) => {
      const found = await info(id);
      return {
        ...found,
        views: found.views.map((v) =>
          v.name === "Board" ? { ...v, group: "left" } : v,
        ),
      };
    });
    vi.spyOn(collections, "query").mockImplementation((input) =>
      query({ ...input, group: "left" }),
    );
    renderApp(`/data/${COLLECTION.issues}?view=Board`, undefined, collections);
    const column = await screen.findByRole("region", { name: /^0/ });
    expect(within(column).queryByRole("button", { name: /^Move / })).toBeNull();
    expect(screen.queryByText(/Drag a card/)).toBeNull();
    const card = within(column).getAllByRole("button")[0];
    expect(card?.getAttribute("draggable")).toBe("false");
    expect(card?.hasAttribute("aria-describedby")).toBe(false);
  });
});
