// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../i18n/i18n";

import { messageOf, useCollections } from "./context";
import { FormError, SidePanel } from "./side-panel";
import type { CollectionInfo, CsvImported } from "./types";

/** The CSV text from a picked file, or what was pasted. */
function CsvSource({
  text,
  onText,
}: {
  readonly text: string;
  readonly onText: (text: string) => void;
}) {
  const t = useT();
  return (
    <>
      <label className="vl-field">
        {t("collection-csv-file")}
        <input
          type="file"
          accept=".csv,text/csv"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) void file.text().then(onText);
          }}
        />
      </label>
      <label className="vl-field">
        {t("collection-csv-paste")}
        <textarea
          rows={6}
          spellCheck={false}
          value={text}
          onChange={(event) => {
            onText(event.target.value);
          }}
        />
      </label>
    </>
  );
}

/** What an import did, or would do. */
function Report({ report }: { readonly report: CsvImported }) {
  const t = useT();
  return (
    <div className="vl-csv-report" role="status">
      <p>
        {t(
          report.written ? "collection-csv-added" : "collection-csv-would-add",
          {
            rows: report.rows,
          },
        )}
      </p>
      {report.skippedColumns.length > 0 && (
        <p>
          {t("collection-csv-skipped", {
            columns: report.skippedColumns.join(", "),
          })}
        </p>
      )}
      {report.diagnostics.length > 0 && (
        <ul>
          {report.diagnostics.slice(0, 20).map((d, index) => (
            <li key={`${String(index)}-${d.location}`}>
              {d.location}: {d.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * Adds a CSV file's rows to a collection: "Check" runs a dry run that
 * reports what would happen; "Import" writes, all or nothing.
 */
export function ImportPanel({
  info,
  onClose,
}: {
  readonly info: CollectionInfo;
  readonly onClose: () => void;
}) {
  const t = useT();
  const store = useCollections();
  const [csv, setCsv] = useState("");
  const [report, setReport] = useState<CsvImported>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const run = (dryRun: boolean) => {
    setBusy(true);
    store
      .importCsv({ collection_id: info.id, csv, dry_run: dryRun })
      .then(
        (result) => {
          setReport(result);
          setError(undefined);
          if (result.written) setCsv("");
        },
        (error_: unknown) => {
          setError(messageOf(error_));
          setReport(undefined);
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <SidePanel
      title={t("collection-import-title", { collection: info.title })}
      onClose={onClose}
    >
      <p className="vl-muted">{t("collection-import-intro")}</p>
      <CsvSource
        text={csv}
        onText={(text) => {
          setCsv(text);
          setReport(undefined);
        }}
      />
      <FormError error={error} />
      {report && <Report report={report} />}
      <div className="vl-panel-actions">
        <button
          type="button"
          className="vl-text-button"
          disabled={busy || csv.trim() === ""}
          onClick={() => {
            run(true);
          }}
        >
          {t("collection-csv-check")}
        </button>
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy || csv.trim() === ""}
          onClick={() => {
            run(false);
          }}
        >
          {t("collection-csv-import")}
        </button>
      </div>
    </SidePanel>
  );
}

/** The folder new collections go in: next to the first one listed. */
export function defaultFolder(paths: readonly string[]): string {
  const first = paths[0];
  if (first === undefined) return "drives/shared";
  const slash = first.lastIndexOf("/");
  return slash > 0 ? first.slice(0, slash) : first;
}

/**
 * Makes a collection: blank (just a title column) or from a CSV file,
 * whose column types the server infers.
 */
export function NewCollectionPanel({
  folder: startFolder,
  onCreated,
  onClose,
}: {
  readonly folder: string;
  readonly onCreated: (collectionId: string) => void;
  readonly onClose: () => void;
}) {
  const t = useT();
  const store = useCollections();
  const [title, setTitle] = useState("");
  const [folder, setFolder] = useState(startFolder);
  const [csv, setCsv] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  return (
    <SidePanel title={t("collection-new")} onClose={onClose}>
      <form
        className="vl-row-fields"
        onSubmit={(event) => {
          event.preventDefault();
          setBusy(true);
          store
            .createFromCsv({
              title: title.trim(),
              folder: folder.trim(),
              // A blank collection is a CSV with only the title column.
              csv: csv.trim() === "" ? "Name\n" : csv,
            })
            .then(
              (result) => {
                onCreated(result.collection.id);
              },
              (error_: unknown) => {
                setError(messageOf(error_));
              },
            )
            .finally(() => {
              setBusy(false);
            });
        }}
      >
        <label className="vl-field">
          {t("collection-new-title")}
          <input
            required
            maxLength={100}
            value={title}
            onChange={(event) => {
              setTitle(event.target.value);
            }}
          />
        </label>
        <label className="vl-field">
          {t("collection-new-folder")}
          <input
            required
            value={folder}
            spellCheck={false}
            onChange={(event) => {
              setFolder(event.target.value);
            }}
          />
        </label>
        <p className="vl-muted vl-hint">{t("collection-new-csv-hint")}</p>
        <CsvSource text={csv} onText={setCsv} />
        <FormError error={error} />
        <div className="vl-panel-actions">
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("collection-create")}
          </button>
        </div>
      </form>
    </SidePanel>
  );
}
