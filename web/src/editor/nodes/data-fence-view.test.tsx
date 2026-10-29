// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { Icon } from "../../shell/icon";
import { body, fence } from "../content";
import { renderEditor } from "../test-editor";

const headIcon = () => document.querySelector(".vl-fence-head path")?.getAttribute("d");

const preview = () => {
  const element = document.querySelector<HTMLElement>(".vl-fence-preview");
  if (!element) {
    throw new Error("no preview");
  }
  return within(element);
};

describe("DataFenceView", () => {
  it("draws a bar chart with an accessible summary", async () => {
    await renderEditor(
      body(fence("chart", "title: Requests\nlabels: [W1, W2]\nvalues: [10, 20]\n")),
    );
    const chart = preview().getByRole("img", { name: "Requests: W1 10, W2 20" });
    expect(chart.querySelectorAll("rect")).toHaveLength(2);
    expect(preview().getByText("Requests").tagName).toBe("FIGCAPTION");
  });

  it.each(["line", "area"])("draws a %s chart as a polyline", async (type) => {
    await renderEditor(body(fence("chart", `type: ${type}\nvalues: [1, 3, 2]\n`)));
    const chart = preview().getByRole("img");
    expect(chart.querySelector("polyline")).not.toBeNull();
    expect(chart.querySelector("rect")).toBeNull();
    expect(document.querySelector("figcaption")).toBeNull();
  });

  it("reads pie-style label lines as the series", async () => {
    await renderEditor(body(fence("chart", "Search: 60\nAds: 40\n")));
    expect(preview().getByRole("img", { name: ": Search 60, Ads 40" })).toBeDefined();
    expect(preview().getByText("Search")).toBeDefined();
  });

  it("says so when a chart has no values", async () => {
    await renderEditor(body(fence("chart", "title: Empty\n")));
    expect(preview().getByText("The chart needs a values list.")).toBeDefined();
  });

  it("previews a table fence as a CSV table with column headers", async () => {
    await renderEditor(body(fence("table", "Name, Value\nAlpha, 1\nBeta, 2\n")));
    const table = preview().getByRole("table");
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((cell) => cell.textContent),
    ).toEqual(["Name", "Value"]);
    expect(
      within(table)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["Alpha", "1", "Beta", "2"]);
  });

  it("previews an empty table fence as an empty table", async () => {
    await renderEditor(body({ type: "dataFence", attrs: { lang: "table", args: "" } }));
    expect(preview().queryAllByRole("columnheader")).toEqual([]);
  });

  it("explains Kroki diagrams and renderer-drawn fences", async () => {
    await renderEditor(body(fence("d2", "a -> b"), fence("timeline", "Oct 6 | Start")));
    expect(screen.getByText(/The d2 diagram is drawn by the diagram service/)).toBeDefined();
    expect(
      screen.getByText("The timeline block is drawn by the renderer when the page is saved."),
    ).toBeDefined();
  });

  it("keeps the source editable under the preview", async () => {
    await renderEditor(body(fence("mermaid", "flowchart LR")));
    expect(screen.getByLabelText("mermaid source").textContent).toBe("flowchart LR");
    expect(document.querySelector(".vl-fence-lang")?.textContent).toBe("mermaid");
  });

  it("re-draws the preview as the source changes", async () => {
    const { editor } = await renderEditor(body(fence("chart", "values: [1]\n")));
    act(() => {
      editor.view.dispatch(editor.state.tr.insertText("labels: [Only]\n", 1));
    });
    expect(await preview().findByRole("img", { name: ": Only 1" })).toBeDefined();
  });

  it("saves its options from the header field", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(body(fence("table", "a\n")));
    await user.type(screen.getByRole("textbox", { name: "table options" }), "sortable");
    expect(editor.getJSON().content[0]?.attrs?.["args"]).toBe("sortable");
  });

  it("makes its options read-only in a read-only editor", async () => {
    await renderEditor(body(fence("board", "## To do\n")), { editable: false });
    const options = screen.getByRole("textbox", { name: "board options" });
    expect((options as HTMLInputElement).readOnly).toBe(true);
  });

  it("shows the schema's icon for its language, and a generic one otherwise", async () => {
    const iconPath = (name: string) => {
      const { container, unmount } = render(<Icon name={name} />);
      const path = container.querySelector("path")?.getAttribute("d");
      unmount();
      return path;
    };
    const { editor } = await renderEditor(body(fence("chart", "values: [1]")));
    expect(headIcon()).toBe(iconPath("bar_chart"));
    act(() => {
      editor.commands.command(({ tr }) => {
        tr.setNodeAttribute(0, "lang", "sparkline");
        return true;
      });
    });
    await screen.findByText("sparkline");
    expect(headIcon()).toBe(iconPath("data_object"));
  });
});
