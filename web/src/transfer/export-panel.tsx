// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";

import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import {
  exportFileName,
  type ExportFormat,
  type ExportTarget,
  type Paper,
  type TransferApi,
} from "./api";

import "./transfer.css";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** An SVG page as an image source; images cannot run scripts. */
const svgSource = (svg: string) => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;

/** Saves a blob through a temporary link. */
function save(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 0);
}

/** Props of {@link ExportPanel}. */
export interface ExportPanelProperties {
  readonly target: ExportTarget;
  readonly api: TransferApi;
  readonly onClose: () => void;
}

/**
 * The export side panel: format and paper, a preview of the PDF's first
 * pages, and the download.
 */
export function ExportPanel({ target, api, onClose }: ExportPanelProperties) {
  const t = useT();
  const headingId = useId();
  const formats: ExportFormat[] =
    target.kind === "page" ? ["pdf", "docx", "html", "md"] : ["pdf", "docx", "html"];
  const [format, setFormat] = useState<ExportFormat>("pdf");
  const [paper, setPaper] = useState<Paper>("a4");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string>();
  const targetKey = JSON.stringify(target);

  // The target is compared by value, so a new object for the same target does not reload.
  const preview = useLoaded(() => api.preview(target, paper), [api, targetKey, paper], errorText);

  const download = async () => {
    setBusy(true);
    setFailure(undefined);
    try {
      save(await api.download(target, format, paper), exportFileName(target.title, format));
    } catch (error) {
      setFailure(errorText(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="vl-transfer-panel" role="dialog" aria-modal="false" aria-labelledby={headingId}>
      <div className="vl-transfer-head">
        <h2 id={headingId}>{t("export-title")}</h2>
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("export-close")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </div>
      <p className="vl-muted">{target.title}</p>
      <fieldset className="vl-transfer-choice">
        <legend>{t("export-format")}</legend>
        {formats.map((value) => (
          <label key={value}>
            <input
              type="radio"
              name="export-format"
              value={value}
              checked={format === value}
              onChange={() => {
                setFormat(value);
              }}
            />
            {t(`export-format-${value}`)}
          </label>
        ))}
      </fieldset>
      {format !== "md" && (
        <fieldset className="vl-transfer-choice">
          <legend>{t("export-paper")}</legend>
          {(["a4", "letter"] as const).map((value) => (
            <label key={value}>
              <input
                type="radio"
                name="export-paper"
                value={value}
                checked={paper === value}
                onChange={() => {
                  setPaper(value);
                }}
              />
              {t(`export-paper-${value}`)}
            </label>
          ))}
        </fieldset>
      )}
      <button
        type="button"
        className="vl-filled-button"
        disabled={busy}
        onClick={() => void download()}
      >
        <Icon name="download" size={20} />
        {busy ? t("export-working") : t("export-download")}
      </button>
      {failure !== undefined && (
        <p className="vl-field-error" role="alert">
          {failure}
        </p>
      )}
      <section aria-label={t("export-preview")} className="vl-transfer-preview">
        <h3>{t("export-preview")}</h3>
        {preview.state === "loading" && (
          <p className="vl-muted" role="status">
            {t("export-preview-loading")}
          </p>
        )}
        {preview.state === "failed" && (
          <p className="vl-field-error" role="alert">
            {preview.message}
          </p>
        )}
        {preview.state === "ready" && (
          <>
            {preview.value.warnings.length > 0 && (
              <details className="vl-transfer-warnings">
                <summary>{t("export-warnings", { count: preview.value.warnings.length })}</summary>
                <ul>
                  {preview.value.warnings.map((w, index) => (
                    <li key={index}>
                      <strong>{w.location}</strong>: {w.message}
                    </li>
                  ))}
                </ul>
              </details>
            )}
            <ol className="vl-transfer-pages">
              {preview.value.pages.map((svg, index) => (
                <li key={index}>
                  <img
                    src={svgSource(svg)}
                    alt={t("export-page-alt", { page: index + 1 })}
                    loading="lazy"
                  />
                </li>
              ))}
            </ol>
            {preview.value.page_count > preview.value.pages.length && (
              <p className="vl-muted">
                {t("export-more-pages", {
                  count: preview.value.page_count - preview.value.pages.length,
                })}
              </p>
            )}
          </>
        )}
      </section>
    </div>
  );
}
