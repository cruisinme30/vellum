// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { AppProviders } from "../app";
import { createMockCollectionsStore } from "../collections/mock";
import { createNorthstarStore } from "../data/mock/northstar";
import { createTestCollab } from "../test/render-app";

import type { ImportResult, IssueImportReport, TransferApi } from "./api";
import { ExportButton } from "./export-button";
import { ImportScreen } from "./import-screen";

const ENG = "01JB3K0000SPACE0ENG0000001";
const PRODUCT = "01JB3K0000SPACE0PRD0000002";

function fakeApi(overrides: Partial<TransferApi> = {}): TransferApi {
  return {
    preview: vi.fn(() =>
      Promise.resolve({ title: "", pages: [], page_count: 0, warnings: [] }),
    ),
    download: vi.fn(() => Promise.resolve(new Blob())),
    importFile: vi.fn(),
    importIssues: vi.fn(),
    ...overrides,
  };
}

/**
 * Renders `element` with the Northstar demo and, unless absent, a transfer API.
 */
function renderWith(element: ReactNode, transfer?: TransferApi) {
  const store = createNorthstarStore();
  const refresh = vi.fn();
  Object.assign(store, { refresh });
  const result = render(
    <AppProviders
      store={store}
      collab={createTestCollab()}
      collections={createMockCollectionsStore()}
      transfer={transfer}
    >
      <MemoryRouter>{element}</MemoryRouter>
    </AppProviders>,
  );
  return { ...result, refresh, user: userEvent.setup() };
}

/** The page import form and the issue import form, in that order. */
function forms(container: HTMLElement) {
  const [pages, issues] = container.querySelectorAll("form");
  if (pages === undefined || issues === undefined)
    throw new Error("two forms expected");
  return { pages: within(pages), issues: within(issues), pageForm: pages };
}

/**
 * Submits the page form. jsdom does not see files set by `user.upload` when it
 * checks the `required` file input, so the form is submitted directly.
 */
function submitPages(form: HTMLFormElement) {
  act(() => {
    fireEvent.submit(form);
  });
}

/** A promise the test settles when it is ready. */
function deferred<T>() {
  const settle: { resolve?: (value: T) => void } = {};
  const promise = new Promise<T>((resolve) => {
    settle.resolve = resolve;
  });
  return {
    promise,
    resolve: (value: T) => {
      settle.resolve?.(value);
    },
  };
}

const RESULT: ImportResult = {
  folder_id: "f 1",
  title: "Team wiki",
  space_id: "01JB3K0000SPACE0PRD0000002",
  nodes: 12,
  files: 1,
  warnings: [{ location: "Roadmap.md", message: "an embed became a link" }],
};

describe("import screen", () => {
  it("asks for a server in demo mode", () => {
    renderWith(<ImportScreen />);
    expect(screen.getByText(/needs a Vellum server/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Import" })).toBeNull();
  });

  it("imports a Word file into the chosen space and links the new folder", async () => {
    const pending = deferred<ImportResult>();
    const importFile = vi.fn<TransferApi["importFile"]>(() => pending.promise);
    const { container, refresh, user } = renderWith(
      <ImportScreen />,
      fakeApi({ importFile }),
    );
    const { pages, pageForm } = forms(container);
    const submit = pages.getByRole("button", { name: "Import" });
    expect(submit).toHaveProperty("disabled", true);

    const file = new File(["PK"], "Report.docx");
    await user.upload(
      pages.getByLabelText("File (.zip export or .docx)"),
      file,
    );
    expect(pages.getByLabelText("Exported from")).toHaveProperty(
      "value",
      "docx",
    );
    await user.selectOptions(pages.getByLabelText("Into"), PRODUCT);
    expect(pages.getByLabelText("Folder")).toHaveProperty("value", "");
    expect(submit).toHaveProperty("disabled", false);
    submitPages(pageForm);
    expect(importFile).toHaveBeenCalledWith(file, {
      space: PRODUCT,
      parent: undefined,
      source: "docx",
    });
    expect(pages.getByRole("button", { name: "Importing…" })).toHaveProperty(
      "disabled",
      true,
    );

    pending.resolve(RESULT);
    expect(
      await pages.findByText(/Imported 12 items into “Team wiki”/),
    ).toBeTruthy();
    expect(refresh).toHaveBeenCalled();
    expect(
      pages.getByRole("link", { name: "Open the folder" }).getAttribute("href"),
    ).toBe("/p/f%201");
    expect(pages.getByText("Roadmap.md")).toBeTruthy();
    expect(pages.getByText(/an embed became a link/)).toBeTruthy();
  });

  it(
    "keeps the picked source when the file name says nothing, and shows no " +
      "warnings when there are none",
    async () => {
      const importFile = vi.fn<TransferApi["importFile"]>(() =>
        Promise.resolve({ ...RESULT, warnings: [] }),
      );
      const { container, user } = renderWith(
        <ImportScreen />,
        fakeApi({ importFile }),
      );
      const { pages, pageForm } = forms(container);
      await user.selectOptions(
        pages.getByLabelText("Exported from"),
        "obsidian",
      );
      const file = new File(["PK"], "vault.zip");
      await user.upload(
        pages.getByLabelText("File (.zip export or .docx)"),
        file,
      );
      submitPages(pageForm);
      expect(importFile).toHaveBeenCalledWith(file, {
        space: ENG,
        parent: undefined,
        source: "obsidian",
      });
      expect(await pages.findByText(/Imported 12 items/)).toBeTruthy();
      expect(container.querySelector(".vl-transfer-warnings")).toBeNull();
    },
  );

  it("shows why an import failed, from an error or anything else", async () => {
    const importFile = vi
      .fn<TransferApi["importFile"]>()
      .mockRejectedValueOnce(new Error("the zip is too large"))
      .mockRejectedValueOnce("scanner down");
    const { container, user } = renderWith(
      <ImportScreen />,
      fakeApi({ importFile }),
    );
    const { pages, pageForm } = forms(container);
    await user.upload(
      pages.getByLabelText("File (.zip export or .docx)"),
      new File(["PK"], "Notion-export.zip"),
    );
    submitPages(pageForm);
    const alert = await pages.findByRole("alert");
    expect(alert.textContent).toBe("the zip is too large");
    submitPages(pageForm);
    expect(await pages.findByText("scanner down")).toBeTruthy();
  });

  it("clears the file choice when the picker is emptied", async () => {
    const { container, user } = renderWith(<ImportScreen />, fakeApi());
    const { pages } = forms(container);
    const input = pages.getByLabelText("File (.zip export or .docx)");
    await user.upload(input, new File(["PK"], "a.docx"));
    expect(pages.getByRole("button", { name: "Import" })).toHaveProperty(
      "disabled",
      false,
    );
    await user.upload(input, []);
    expect(pages.getByRole("button", { name: "Import" })).toHaveProperty(
      "disabled",
      true,
    );
  });
});

describe("issue import form", () => {
  const REPORT: IssueImportReport = {
    created: 4,
    skipped: 0,
    links: 1,
    read: 4,
    keys: [],
    warnings: [
      { message: "two columns were ignored" },
      { external_id: "ENG-9", message: "label dropped" },
    ],
    dry_run: false,
  };

  it("imports a Linear CSV without a repository, and lists its notes", async () => {
    const importIssues = vi.fn<TransferApi["importIssues"]>(() =>
      Promise.resolve(REPORT),
    );
    const { container, user } = renderWith(
      <ImportScreen />,
      fakeApi({ importIssues }),
    );
    const { issues } = forms(container);
    expect(issues.queryByLabelText("Repository")).toBeNull();
    const submit = issues.getByRole("button", { name: "Import" });
    const file = new File(["id,title"], "linear.csv", { type: "text/csv" });
    await user.upload(issues.getByLabelText("Export file"), file);
    expect(submit).toHaveProperty("disabled", true);
    await user.type(issues.getByLabelText("Team key"), "  ops ");
    await user.click(submit);
    expect(importIssues).toHaveBeenCalledWith(file, {
      tracker: "linear",
      team: "OPS",
      repo: undefined,
      dryRun: false,
    });
    expect(await issues.findByText(/Created 4 issues/)).toBeTruthy();
    expect(issues.getByText("two columns were ignored")).toBeTruthy();
    expect(issues.getByText(/ENG-9/)).toBeTruthy();
  });

  it("needs a repository for GitHub, and reports a failed import", async () => {
    const importIssues = vi
      .fn<TransferApi["importIssues"]>()
      .mockRejectedValueOnce(new Error("only admins can import issues"))
      .mockResolvedValueOnce({ ...REPORT, warnings: [], dry_run: true });
    const { container, user } = renderWith(
      <ImportScreen />,
      fakeApi({ importIssues }),
    );
    const { issues } = forms(container);
    await user.selectOptions(issues.getByLabelText("Exported from"), "github");
    await user.type(issues.getByLabelText("Team key"), "eng");
    await user.upload(
      issues.getByLabelText("Export file"),
      new File(["[]"], "issues.json"),
    );
    const preview = issues.getByRole("button", { name: "Preview" });
    expect(preview).toHaveProperty("disabled", true);
    await user.type(issues.getByLabelText("Repository"), " northstar/app ");
    await user.click(issues.getByRole("button", { name: "Import" }));
    const alert = await issues.findByRole("alert");
    expect(alert.textContent).toBe("only admins can import issues");

    await user.click(preview);
    expect(importIssues).toHaveBeenLastCalledWith(expect.any(File), {
      tracker: "github",
      team: "ENG",
      repo: "northstar/app",
      dryRun: true,
    });
    expect(await issues.findByText(/4 issues will be created/)).toBeTruthy();
    expect(container.querySelector(".vl-transfer-warnings")).toBeNull();
  });

  it("imports a Jira export, showing the work in progress", async () => {
    const pending = deferred<IssueImportReport>();
    const importIssues = vi.fn<TransferApi["importIssues"]>(
      () => pending.promise,
    );
    const { container, user } = renderWith(
      <ImportScreen />,
      fakeApi({ importIssues }),
    );
    const { issues } = forms(container);
    await user.selectOptions(issues.getByLabelText("Exported from"), "jira");
    await user.type(issues.getByLabelText("Team key"), "web");
    await user.upload(
      issues.getByLabelText("Export file"),
      new File(["{}"], "jira.json"),
    );
    await user.click(issues.getByRole("button", { name: "Import" }));
    expect(issues.getByRole("button", { name: "Importing…" })).toHaveProperty(
      "disabled",
      true,
    );
    pending.resolve({ ...REPORT, warnings: [] });
    expect(await issues.findByText(/Created 4 issues/)).toBeTruthy();
  });
});

describe("export button", () => {
  const PAGE = { kind: "page", id: "p1", title: "Launch plan" } as const;

  it("renders nothing in demo mode", () => {
    const { container } = renderWith(<ExportButton target={PAGE} />);
    expect(container.querySelector("button")).toBeNull();
  });

  it("opens and closes the export panel from an icon or a labelled button", async () => {
    const { user } = renderWith(
      <>
        <ExportButton target={PAGE} />
        <ExportButton target={PAGE} label="Export book" />
      </>,
      fakeApi(),
    );
    const icon = screen.getByRole("button", { name: "Export" });
    expect(icon.getAttribute("aria-expanded")).toBe("false");
    await user.click(icon);
    expect(icon.getAttribute("aria-expanded")).toBe("true");
    expect(
      await screen.findByRole("button", { name: "Download" }),
    ).toBeTruthy();
    await user.click(icon);
    expect(icon.getAttribute("aria-expanded")).toBe("false");

    const labelled = screen.getByRole("button", { name: "Export book" });
    await user.click(labelled);
    expect(labelled.getAttribute("aria-expanded")).toBe("true");
    await user.click(screen.getByRole("button", { name: "Close export" }));
    expect(labelled.getAttribute("aria-expanded")).toBe("false");
  });
});
