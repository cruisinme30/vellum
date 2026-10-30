// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { body, details, p } from "../content";
import { renderEditor } from "../test-editor";

const content = body(details("Background", p("Hidden until opened")));

describe("DetailsView", () => {
  it("starts collapsed with its body hidden", async () => {
    await renderEditor(content);
    const toggle = screen.getByRole("button", { name: "Expand section" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    const panel = document.querySelector(
      `#${CSS.escape(toggle.getAttribute("aria-controls") ?? "")}`,
    );
    expect(panel?.textContent).toBe("Hidden until opened");
    expect(panel?.hasAttribute("hidden")).toBe(true);
  });

  it("opens and closes from its toggle button", async () => {
    const user = userEvent.setup();
    await renderEditor(content);
    await user.click(screen.getByRole("button", { name: "Expand section" }));
    const toggle = screen.getByRole("button", { name: "Collapse section" });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(
      screen.getByText("Hidden until opened").closest("[hidden]"),
    ).toBeNull();
    await user.click(toggle);
    expect(
      screen.getByRole("button", { name: "Expand section" }),
    ).toBeDefined();
  });

  it("does not save the open state in the document", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(content);
    const before = editor.getJSON();
    await user.click(screen.getByRole("button", { name: "Expand section" }));
    expect(editor.getJSON()).toEqual(before);
  });

  it("saves an edited summary to the node", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(content);
    const summary = screen.getByRole("textbox", { name: "Section title" });
    await user.clear(summary);
    await user.type(summary, "Context");
    expect(editor.getJSON().content[0]?.attrs?.["summary"]).toBe("Context");
  });

  it("makes the summary read-only in a read-only editor", async () => {
    await renderEditor(content, { editable: false });
    const summary = screen.getByRole("textbox", { name: "Section title" });
    expect((summary as HTMLInputElement).readOnly).toBe(true);
  });
});
