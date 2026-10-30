// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ApiError } from "../../api/http";
import type { RenderApi } from "../../render/api";
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

const drawn = (html: string, warnings: string[] = []) => ({
  html,
  warnings,
  stale: false,
  render_version: 2,
});

/** A renderer whose blocks come from `block`; the spy is handed back to assert on. */
function fakeRender(block: RenderApi["block"]) {
  const spy = vi.fn(block);
  const api: RenderApi = {
    block: spy,
    page: vi.fn(),
    stylesheet: vi.fn(() => Promise.resolve({ css: "", scope: "vl-render", render_version: 2 })),
  };
  return { api, spy };
}

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
      screen.getByText("The timeline block is drawn when Vellum runs with a server."),
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

  describe("with a server", () => {
    it("shows the server's drawing, with the source behind the Source button", async () => {
      const user = userEvent.setup();
      const { api, spy } = fakeRender(() =>
        Promise.resolve(
          drawn('<article class="vl-doc"><ol class="vl-timeline"><li>Kickoff</li></ol></article>'),
        ),
      );
      await renderEditor(body(fence("timeline", "2026-01-10 | Kickoff\n")), { render: api });
      expect(await preview().findByText("Kickoff")).toBeDefined();
      expect(spy).toHaveBeenCalledWith({
        lang: "timeline",
        args: "",
        source: "2026-01-10 | Kickoff\n",
      });
      expect(document.querySelector(".vl-fence-render")?.classList.contains("vl-render")).toBe(
        true,
      );
      const source = screen.getByLabelText("timeline source");
      expect(source.hidden).toBe(true);
      const toggle = screen.getByRole("button", { name: "Source" });
      expect(toggle.getAttribute("aria-pressed")).toBe("false");
      await user.click(toggle);
      expect(source.hidden).toBe(false);
      expect(toggle.getAttribute("aria-pressed")).toBe("true");
    });

    it("opens the source of an empty block, to write it", async () => {
      const { api } = fakeRender(() => Promise.resolve(drawn("<article></article>")));
      await renderEditor(body({ type: "dataFence", attrs: { lang: "mermaid", args: "" } }), {
        render: api,
      });
      expect(screen.getByLabelText("mermaid source").hidden).toBe(false);
    });

    it("shows render warnings with Copy diagnostic", async () => {
      const { api } = fakeRender(() =>
        Promise.resolve(drawn("<article><pre>x</pre></article>", ["calculator `x`: oops"])),
      );
      await renderEditor(body(fence("calculator", "x: X = y\n")), { render: api });
      expect(await preview().findByText(/calculator `x`: oops/)).toBeDefined();
      expect(preview().getByRole("button", { name: "Copy diagnostic" })).toBeDefined();
    });

    it("falls back to the client preview when the server cannot draw", async () => {
      const { api } = fakeRender(() =>
        Promise.reject(
          new ApiError("unavailable", "The renderer is busy.", {
            requestId: "req-9",
            route: "POST /api/v1/render/block",
          }),
        ),
      );
      await renderEditor(body(fence("chart", "labels: [A]\nvalues: [3]\n")), { render: api });
      expect(
        await preview().findByText(
          "The block could not be drawn: The renderer is busy. (request req-9)",
        ),
      ).toBeDefined();
      expect(preview().getByRole("img", { name: ": A 3" })).toBeDefined();
      expect(preview().getByRole("button", { name: "Copy diagnostic" })).toBeDefined();
    });

    it("keeps collections on the client preview", async () => {
      const { api, spy } = fakeRender(() => Promise.resolve(drawn("<p>server</p>")));
      await renderEditor(body(fence("collection", "source: x\n")), { render: api });
      expect(spy).not.toHaveBeenCalled();
    });
  });
});
