// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n/i18n";
import { renderApp } from "../test/render-app";

import { CollectionsProvider } from "./context";
import { CollectionEmbedPreview as EmbedPreviewForTest } from "./embed-preview";
import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";
import { withNames } from "./view-editor";

const ISSUES = `/data/${COLLECTION.issues}`;

afterEach(() => {
  vi.restoreAllMocks();
});

async function openIssues(path = ISSUES) {
  const collections = createMockCollectionsStore();
  const rendered = renderApp(path, undefined, collections);
  await screen.findByRole("heading", { level: 1, name: "Issues" });
  return { ...rendered, collections };
}

describe("view settings", () => {
  it("shows stored keys as property names", () => {
    const properties = [{ key: "est", name: "Estimate", type: "number", computed: false }] as const;
    expect(withNames("est > 1", properties)).toBe("Estimate > 1");
    expect(withNames("est", properties)).toBe("Estimate");
    expect(withNames("estimate desc", properties)).toBe("estimate desc");
  });

  it("makes a new view and switches to it", async () => {
    const { user, location, collections } = await openIssues();
    const save = vi.spyOn(collections, "saveView");
    await user.click(screen.getByRole("button", { name: "New view" }));
    const panel = screen.getByRole("dialog", { name: "New view" });
    expect(document.activeElement).toBe(within(panel).getByLabelText("Name"));
    await user.type(within(panel).getByLabelText("Name"), "Done work");
    await user.selectOptions(within(panel).getByLabelText("Layout"), "list");
    await user.type(within(panel).getByLabelText("Filters, one per line"), "Status is Done");
    await user.click(within(panel).getByRole("checkbox", { name: "Link" }));
    await user.click(within(panel).getByRole("button", { name: "Save" }));
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        collection_id: COLLECTION.issues,
        view: "Done work",
        kind: "list",
        filter: ["Status is Done"],
        hide: ["link"],
      }),
    );
    expect(await screen.findByRole("tab", { name: "Done work" })).toBeDefined();
    expect(location()).toBe(`${ISSUES}?view=Done+work`);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("edits the open view, says why a save fails, and deletes it", async () => {
    const { user, location, collections } = await openIssues(`${ISSUES}?view=Open`);
    await user.click(screen.getByRole("button", { name: "Edit view" }));
    const panel = screen.getByRole("dialog", { name: "Edit “Open”" });
    expect(within(panel).getByLabelText<HTMLTextAreaElement>("Filters, one per line").value).toBe(
      "Status is not Done",
    );
    vi.spyOn(collections, "saveView").mockRejectedValueOnce(new Error("no property Nope"));
    await user.click(within(panel).getByRole("button", { name: "Save" }));
    const found = await within(panel).findByRole("alert");
    expect(found.textContent).toBe("no property Nope");
    await user.click(within(panel).getByRole("button", { name: "Delete view" }));
    await user.click(within(panel).getByRole("button", { name: /Press again to confirm/ }));
    await waitFor(() => {
      expect(screen.queryByRole("tab", { name: "Open" })).toBeNull();
    });
    expect(location()).toBe(ISSUES);
  });

  it("closes a panel with Escape and gives focus back", async () => {
    const { user } = await openIssues();
    const opener = screen.getByRole("button", { name: "Properties" });
    await user.click(opener);
    expect(screen.getByRole("dialog", { name: "Properties" })).toBeDefined();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});

describe("property settings", () => {
  it("adds a select, renames it and removes a property", async () => {
    const { user, collections } = await openIssues();
    await user.click(screen.getByRole("button", { name: "Properties" }));
    const panel = screen.getByRole("dialog", { name: "Properties" });
    const add = within(panel).getByRole("form", { name: "Add property" });
    await user.type(within(add).getByLabelText("Name"), "Priority");
    await user.selectOptions(within(add).getByLabelText("Type"), "select");
    await user.type(within(add).getByLabelText("Options, separated by commas"), "P0, P1, P0");
    await user.click(within(add).getByRole("button", { name: "Add property" }));
    const field = await within(panel).findByDisplayValue("Priority");
    const found = await collections.info(COLLECTION.issues);
    expect(found.properties.at(-1)).toMatchObject({
      key: "priority",
      options: [{ name: "P0" }, { name: "P1" }],
    });

    await user.clear(field);
    await user.type(field, "Urgency{Enter}");
    expect(await within(panel).findByDisplayValue("Urgency")).toBeDefined();

    await user.click(within(panel).getByRole("button", { name: "Delete Link" }));
    await user.click(within(panel).getByRole("button", { name: /Delete Link\? Press again/ }));
    await waitFor(() => {
      expect(within(panel).queryByDisplayValue("Link")).toBeNull();
    });
    expect(within(panel).queryByRole("button", { name: "Delete Issue" })).toBeNull();
  });

  it("shows the server's reason when a change is refused", async () => {
    const { user, collections } = await openIssues();
    vi.spyOn(collections, "updateProperty").mockRejectedValueOnce(
      new Error('cannot rename "Estimate": Points left uses it; change it first'),
    );
    await user.click(screen.getByRole("button", { name: "Properties" }));
    const panel = screen.getByRole("dialog", { name: "Properties" });
    const estimate = within(panel).getByDisplayValue("Estimate");
    await user.type(estimate, "s{Enter}");
    const found = await within(panel).findByRole("alert");
    expect(found.textContent).toMatch(/Points left uses it/);
  });
});

describe("rows and CSV", () => {
  it("deletes a row after a second press", async () => {
    const { user } = await openIssues();
    const grid = await screen.findByRole("grid");
    const before = within(grid).getAllByRole("row").length;
    const [first] = within(grid).getAllByRole("button", { name: /^Open / });
    if (!first) throw new Error("no rows");
    await user.click(first);
    const panel = await screen.findByRole("dialog");
    await user.click(await within(panel).findByRole("button", { name: "Delete row" }));
    await user.click(within(panel).getByRole("button", { name: /Press again to confirm/ }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    await waitFor(() => {
      expect(within(screen.getByRole("grid")).getAllByRole("row")).toHaveLength(before - 1);
    });
  });

  it("checks a CSV file, then imports it", async () => {
    const { user } = await openIssues();
    await user.click(screen.getByRole("button", { name: "Import CSV" }));
    const panel = screen.getByRole("dialog", { name: "Import into Issues" });
    const check = within(panel).getByRole("button", { name: "Check" });
    expect(check).toHaveProperty("disabled", true);
    const file = new File(["Issue,Mood\nFrom a file,calm\n"], "rows.csv", { type: "text/csv" });
    await user.upload(within(panel).getByLabelText("CSV file"), file);
    await waitFor(() => {
      expect(within(panel).getByLabelText<HTMLTextAreaElement>("Or paste CSV text").value).toMatch(
        /From a file/,
      );
    });
    await user.click(check);
    const found = await within(panel).findByText("One row would be added.");
    expect(found.textContent).toBeTruthy();
    expect(within(panel).getByText("Ignored columns: Mood")).toBeDefined();
    await user.click(within(panel).getByRole("button", { name: "Import" }));
    expect(await within(panel).findByText("Added one row.")).toBeDefined();
    expect(await within(screen.getByRole("grid")).findByText("From a file")).toBeDefined();
  });

  it("exports the open view as a CSV download", async () => {
    const created = vi.fn(() => "blob:issues");
    const revoked = vi.fn();
    Object.assign(URL, { createObjectURL: created, revokeObjectURL: revoked });
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockReturnValue();
    const { user } = await openIssues(`${ISSUES}?view=Open`);
    await user.click(screen.getByRole("button", { name: "Export CSV" }));
    await waitFor(() => {
      expect(click).toHaveBeenCalledOnce();
    });
    const link = click.mock.contexts[0] as HTMLAnchorElement;
    expect(link.download).toBe("Issues - Open.csv");
    expect(link.href).toBe("blob:issues");
    expect(link.isConnected).toBe(false);
    await waitFor(() => {
      expect(revoked).toHaveBeenCalledWith("blob:issues");
    });
  });
});

describe("new collections", () => {
  it("creates a blank collection next to the others and opens it", async () => {
    const { user, location } = renderApp("/data");
    await screen.findByRole("link", { name: /^Issues/ });
    await user.click(screen.getByRole("button", { name: "New collection" }));
    const panel = screen.getByRole("dialog", { name: "New collection" });
    expect(within(panel).getByLabelText<HTMLInputElement>("Drive folder").value).toBe(
      "engineering",
    );
    await user.type(within(panel).getByLabelText("Title"), "Hosts");
    await user.click(within(panel).getByRole("button", { name: "Create" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Hosts" })).toBeDefined();
    expect(location()).toMatch(/^\/data\/\w+$/);
  });

  it("says why a collection could not be made", async () => {
    const { user } = renderApp("/data");
    await screen.findByRole("link", { name: /^Issues/ });
    await user.click(screen.getByRole("button", { name: "New collection" }));
    const panel = screen.getByRole("dialog", { name: "New collection" });
    await user.type(within(panel).getByLabelText("Title"), "issues");
    await user.click(within(panel).getByRole("button", { name: "Create" }));
    const found = await within(panel).findByRole("alert");
    expect(found.textContent).toMatch(/already there/);
  });
});

describe("collection blocks in the editor", () => {
  it("previews the block's live rows, and says what is missing", async () => {
    const store = createMockCollectionsStore();
    const { rerender } = render(
      <I18nProvider>
        <CollectionsProvider store={store}>
          <EmbedPreviewForTest fence={`id: ${COLLECTION.issues}\nview: Open`} />
        </CollectionsProvider>
      </I18nProvider>,
    );
    const figure = await screen.findByRole("figure");
    expect(figure.textContent).toMatch(/^Issues, Open: \d+ rows/);
    expect(within(figure).getAllByRole("columnheader")[0]?.textContent).toBe("Issue");
    rerender(
      <I18nProvider>
        <CollectionsProvider store={store}>
          <EmbedPreviewForTest fence="view: Open" />
        </CollectionsProvider>
      </I18nProvider>,
    );
    expect(screen.getByText(/Add an “id:” line/)).toBeDefined();
    rerender(
      <I18nProvider>
        <CollectionsProvider store={store}>
          <EmbedPreviewForTest fence="id: nope" />
        </CollectionsProvider>
      </I18nProvider>,
    );
    expect(await screen.findByText("collection nope not found")).toBeDefined();
  });

  it("falls back to the renderer note outside the app", () => {
    render(
      <I18nProvider>
        <EmbedPreviewForTest fence="id: x" />
      </I18nProvider>,
    );
    expect(screen.getByText(/drawn by the renderer/)).toBeDefined();
  });
});
