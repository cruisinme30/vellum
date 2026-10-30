// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Export (`/api/v1/export`) and import (`/api/v1/import`) over a Vellum server. */

import type { Http } from "../api/http";

/** Formats a page or book exports to. Books have no Markdown export. */
export type ExportFormat = "pdf" | "docx" | "html" | "md";
export type Paper = "a4" | "letter";

/** What to export: a page, or a book (whole, one section or a subtree). */
export type ExportTarget =
  | { readonly kind: "page"; readonly id: string; readonly title: string }
  | {
      readonly kind: "book";
      readonly id: string;
      readonly title: string;
      readonly scope: "whole" | "section" | "subtree";
      readonly section?: string | undefined;
    };

interface ExportWarning {
  readonly location: string;
  readonly message: string;
}

/** A PDF's first pages, drawn as SVG documents. */
interface ExportPreview {
  readonly title: string;
  readonly pages: readonly string[];
  readonly page_count: number;
  readonly warnings: readonly ExportWarning[];
}

/** Where an import comes from. */
export type ImportSource =
  "notion" | "confluence" | "docx" | "markdown" | "obsidian" | "outline" | "docmost";

export interface ImportResult {
  readonly folder_id: string;
  readonly title: string;
  readonly space_id: string;
  readonly nodes: number;
  readonly files: number;
  readonly warnings: readonly ExportWarning[];
}

/** The export and import operations the web app uses. */
export interface TransferApi {
  preview(target: ExportTarget, paper: Paper): Promise<ExportPreview>;
  download(target: ExportTarget, format: ExportFormat, paper: Paper): Promise<Blob>;
  importFile(
    file: File,
    into: {
      readonly space: string;
      readonly parent?: string | undefined;
      readonly source: ImportSource;
    },
  ): Promise<ImportResult>;
}

const EXPORT = "/api/v1/export";
const segment = encodeURIComponent;

function exportPath(target: ExportTarget): string {
  return target.kind === "page"
    ? `${EXPORT}/pages/${segment(target.id)}`
    : `${EXPORT}/books/${segment(target.id)}`;
}

function exportQuery(target: ExportTarget, format: string, paper: Paper) {
  return target.kind === "page"
    ? { format, paper }
    : { format, paper, scope: target.scope, section: target.section };
}

/** Export and import over a Vellum server. */
export function createHttpTransferApi(http: Http): TransferApi {
  return {
    preview: (target, paper) =>
      http.get<ExportPreview>(exportPath(target), exportQuery(target, "preview", paper)),
    download: (target, format, paper) =>
      http.blob(exportPath(target), exportQuery(target, format, paper)),
    importFile: (file, into) =>
      http.upload<ImportResult>(
        "/api/v1/import",
        { space: into.space, parent: into.parent, source: into.source, name: file.name },
        file,
      ),
  };
}

/** A file name for an export: the title without characters file systems refuse. */
export function exportFileName(title: string, format: ExportFormat): string {
  const refused = String.raw`/\:*?"<>|`;
  let clean = "";
  for (const char of title) {
    clean += char < " " || refused.includes(char) ? "-" : char;
  }
  const stem = clean.trim().slice(0, 120);
  return `${stem === "" ? "export" : stem}.${format}`;
}

/** The import source a file's name suggests. */
export function guessSource(name: string): ImportSource | undefined {
  const lower = name.toLowerCase();
  if (lower.endsWith(".docx")) return "docx";
  if (lower.includes("notion")) return "notion";
  if (lower.includes("confluence")) return "confluence";
  return undefined;
}
