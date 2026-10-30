// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, fireEvent, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { renderWithProviders } from "../test/render-app";

import { AppKeyboard } from "./app-keyboard";

function key(name: string, extra: Partial<KeyboardEventInit> = {}) {
  act(() => {
    fireEvent.keyDown(document.body, { key: name, ...extra });
  });
}

describe("AppKeyboard", () => {
  it("finds pages by title in the ⌘K menu, marks the match and opens one", () => {
    const { location, store } = renderWithProviders(
      <AppKeyboard>
        <p>app</p>
      </AppKeyboard>,
    );
    key("k", { ctrlKey: true });
    const search = screen.getByRole("combobox", { name: "Search pages and commands" });
    fireEvent.change(search, { target: { value: "gateway arch" } });
    const [first] = screen.getAllByRole("option");
    expect(first?.textContent).toContain("Gateway architecture");
    const marks = [...(first?.querySelectorAll("mark") ?? [])].map((mark) => mark.textContent);
    expect(marks).toEqual(["Gateway", "arch"]);
    fireEvent.keyDown(search, { key: "Enter" });
    const gateway = store.getSnapshot().pages.find((page) => page.title === "Gateway architecture");
    expect(location()).toBe(`/p/${gateway?.id ?? ""}`);
  });

  it("offers full-text search for what was typed", () => {
    const { location } = renderWithProviders(
      <AppKeyboard>
        <p>app</p>
      </AppKeyboard>,
    );
    key("k", { ctrlKey: true });
    const search = screen.getByRole("combobox");
    fireEvent.change(search, { target: { value: "rate limits" } });
    fireEvent.mouseDown(
      screen.getByRole("option", { name: "Search every page for “rate limits”" }),
    );
    expect(location()).toBe("/search?q=rate%20limits");
  });

  it("goes to My Work with G then M, and has no quick-add without a server", () => {
    const { location } = renderWithProviders(
      <AppKeyboard>
        <p>app</p>
      </AppKeyboard>,
    );
    key("g");
    key("m");
    expect(location()).toBe("/my-work");
    key("q");
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
