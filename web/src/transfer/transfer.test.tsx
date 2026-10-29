// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { I18nProvider } from "../i18n/i18n";

import {
  createHttpTransferApi,
  exportFileName,
  guessSource,
  type ExportTarget,
  type TransferApi,
} from "./api";
import { ExportPanel } from "./export-panel";
import { IssueImportForm } from "./import-screen";

const PAGE: ExportTarget = { kind: "page", id: "p 1", title: "Launch plan" };
const SVG = '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>';

function fakeApi(overrides: Partial<TransferApi> = {}): TransferApi {
  return {
    preview: vi.fn(() =>
      Promise.resolve({
        title: "Launch plan",
        pages: [SVG, SVG],
        page_count: 14,
        warnings: [{ location: "chart", message: "drawn as a table" }],
      }),
    ),
    download: vi.fn(() => Promise.resolve(new Blob(["%PDF-"]))),
    importFile: vi.fn(),
    importIssues: vi.fn(),
    ...overrides,
  };
}

describe("transfer API", () => {
  it("calls the export and import routes", async () => {
    const get = vi.fn(() => Promise.resolve({}));
    const blob = vi.fn(() => Promise.resolve(new Blob()));
    const upload = vi.fn(() => Promise.resolve({}));
    const api = createHttpTransferApi({ get, blob, upload } as unknown as Http);
    await api.preview(PAGE, "letter");
    expect(get).toHaveBeenCalledWith("/api/v1/export/pages/p%201", {
      format: "preview",
      paper: "letter",
    });
    await api.download(
      { kind: "book", id: "b1", title: "Handbook", scope: "subtree", section: "s2" },
      "docx",
      "a4",
    );
    expect(blob).toHaveBeenCalledWith("/api/v1/export/books/b1", {
      format: "docx",
      paper: "a4",
      scope: "subtree",
      section: "s2",
    });
    const file = new File(["PK"], "Team wiki.zip");
    await api.importFile(file, { space: "s1", source: "notion" });
    expect(upload).toHaveBeenCalledWith(
      "/api/v1/import",
      { space: "s1", parent: undefined, source: "notion", name: "Team wiki.zip" },
      file,
    );
  });

  it("names files safely and guesses sources", () => {
    expect(exportFileName("Q3 plan: v2/final", "pdf")).toBe("Q3 plan- v2-final.pdf");
    expect(exportFileName("  ", "md")).toBe("export.md");
    expect(guessSource("Report.DOCX")).toBe("docx");
    expect(guessSource("Notion-export-abc.zip")).toBe("notion");
    expect(guessSource("vault.zip")).toBeUndefined();
  });
});

describe("export panel", () => {
  it("shows the preview pages as images and downloads the chosen format", async () => {
    const user = userEvent.setup();
    const download = vi.fn(() => Promise.resolve(new Blob(["%PDF-"])));
    const preview = vi.fn<TransferApi["preview"]>(() =>
      Promise.resolve({
        title: "Launch plan",
        pages: [SVG, SVG],
        page_count: 14,
        warnings: [{ location: "chart", message: "drawn as a table" }],
      }),
    );
    const api = fakeApi({ download, preview });
    const createObjectURL = vi.fn(() => "blob:x");
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL, revokeObjectURL }));
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(vi.fn());
    render(
      <I18nProvider>
        <ExportPanel target={PAGE} api={api} onClose={vi.fn()} />
      </I18nProvider>,
    );
    const pages = await screen.findAllByRole("img");
    expect(pages).toHaveLength(2);
    expect(pages[0]?.getAttribute("src")).toMatch(/^data:image\/svg\+xml/);
    expect(screen.getByText(/12 more pages/)).toBeTruthy();
    expect(screen.getByText(/One block exports differently/)).toBeTruthy();

    await user.click(screen.getByLabelText("Word"));
    await user.click(screen.getByLabelText("US Letter"));
    await user.click(screen.getByRole("button", { name: "Download" }));
    expect(download).toHaveBeenCalledWith(PAGE, "docx", "letter");
    expect(click).toHaveBeenCalled();
    expect(preview).toHaveBeenLastCalledWith(PAGE, "letter");
    click.mockRestore();
    vi.unstubAllGlobals();
  });

  it("shows why an export failed", async () => {
    const user = userEvent.setup();
    const api = fakeApi({
      preview: () => Promise.reject(new Error("the server is busy")),
      download: () => Promise.reject(new Error("too large to export")),
    });
    render(
      <I18nProvider>
        <ExportPanel target={PAGE} api={api} onClose={vi.fn()} />
      </I18nProvider>,
    );
    expect(await screen.findByText("the server is busy")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Download" }));
    expect(await screen.findByText("too large to export")).toBeTruthy();
  });
});

describe("issue import", () => {
  it("previews, then imports, a GitHub export", async () => {
    const user = userEvent.setup();
    const importIssues = vi.fn<TransferApi["importIssues"]>((_file, into) =>
      Promise.resolve({
        created: 2,
        skipped: 1,
        links: 0,
        read: 3,
        keys: [],
        warnings: [{ external_id: "northstar/app#7", message: "assignee left unassigned" }],
        dry_run: into.dryRun,
      }),
    );
    render(
      <I18nProvider>
        <IssueImportForm api={fakeApi({ importIssues })} />
      </I18nProvider>,
    );
    const preview = screen.getByRole("button", { name: "Preview" });
    expect(preview).toHaveProperty("disabled", true);
    await user.selectOptions(screen.getByLabelText("Exported from"), "github");
    await user.type(screen.getByLabelText("Repository"), "northstar/app");
    await user.type(screen.getByLabelText("Team key"), "eng");
    const file = new File(["[]"], "issues.json", { type: "application/json" });
    await user.upload(screen.getByLabelText("Export file"), file);

    await user.click(preview);
    expect(importIssues).toHaveBeenLastCalledWith(file, {
      tracker: "github",
      team: "ENG",
      repo: "northstar/app",
      dryRun: true,
    });
    expect(await screen.findByText(/2 issues will be created/)).toBeTruthy();
    expect(screen.getByText("One note")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Import" }));
    expect(await screen.findByText(/Created 2 issues/)).toBeTruthy();
  });
});
