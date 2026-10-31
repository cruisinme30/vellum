// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n/i18n";

import { NewButton, type NewActions } from "./new-menu";

function renderNew(actions: NewActions) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <NewButton actions={actions} className="vl-fab">
        New
      </NewButton>
    </I18nProvider>,
  );
  return user;
}

describe("New", () => {
  it("makes a page at once where only pages can be made", async () => {
    const page = vi.fn();
    const user = renderNew({ page });
    const button = screen.getByRole("button", { name: "New" });
    expect(button.hasAttribute("aria-haspopup")).toBe(false);
    await user.click(button);
    expect(page).toHaveBeenCalledOnce();
  });

  it("offers a page or a book where books can be made (#566)", async () => {
    const page = vi.fn();
    const book = vi.fn();
    const user = renderNew({ page, book });
    const button = screen.getByRole("button", { name: "New" });
    expect(button.getAttribute("aria-haspopup")).toBe("menu");
    await user.click(button);
    const menu = screen.getByRole("menu", { name: "Make something new" });
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement?.textContent).toBe("Page");
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement?.textContent).toBe("Book");
    await user.keyboard("{Enter}");
    expect(book).toHaveBeenCalledOnce();
    expect(page).not.toHaveBeenCalled();
    expect(menu.isConnected).toBe(false);

    // Escape closes the menu and gives focus back to New.
    await user.click(button);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(button);
    await user.click(button);
    await user.click(screen.getByRole("menuitem", { name: "Page" }));
    expect(page).toHaveBeenCalledOnce();
  });
});
