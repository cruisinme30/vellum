// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";

import { failureText } from "../data/failure";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import type { Attachment } from "./api";
import { usePageFiles, type PageFiles } from "./context";
import { UploadButton } from "./upload-button";

import "./attachments.css";

const SIZE = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

/** A byte count as a short size. */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`;
  if (bytes < 1024 ** 2) return `${SIZE.format(bytes / 1024)} KB`;
  return `${SIZE.format(bytes / 1024 ** 2)} MB`;
}

function FileRow({
  file,
  canWrite,
  onDelete,
}: {
  readonly file: Attachment;
  readonly canWrite: boolean;
  readonly onDelete: () => void;
}) {
  const t = useT();
  const [asking, setAsking] = useState(false);
  return (
    <li>
      <a href={file.url} download={file.name}>
        {file.name}
      </a>
      <small className="vl-muted">{sizeText(file.size)}</small>
      {canWrite && !asking && (
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("files-delete", { name: file.name })}
          title={t("files-delete", { name: file.name })}
          onClick={() => {
            setAsking(true);
          }}
        >
          <Icon name="delete" size={20} />
        </button>
      )}
      {asking && (
        <span className="vl-files-confirm">
          <button type="button" className="vl-text-button" onClick={onDelete}>
            {t("files-delete-confirm", { name: file.name })}
          </button>
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setAsking(false);
            }}
          >
            {t("files-cancel")}
          </button>
        </span>
      )}
    </li>
  );
}

function FileList({
  files,
  canWrite,
}: {
  readonly files: PageFiles;
  readonly canWrite: boolean;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [notice, setNotice] = useState("");
  const [problem, setProblem] = useState<string>();
  const loaded = useLoaded(
    () => files.api.list(files.pageId),
    [files, version],
    failureText,
  );
  const reload = () => {
    setVersion((value) => value + 1);
  };
  const remove = async (file: Attachment) => {
    setProblem(undefined);
    try {
      await files.api.remove(file.url);
      setNotice(t("files-deleted", { name: file.name }));
      reload();
    } catch (error) {
      setProblem(failureText(error));
    }
  };
  return (
    <>
      {canWrite && (
        <UploadButton
          label={t("files-upload")}
          onUploaded={(file) => {
            setNotice(t("files-uploaded", { name: file.name }));
            reload();
          }}
        />
      )}
      <p role="status" className="vl-files-notice">
        {notice}
      </p>
      {problem !== undefined && (
        <p className="vl-field-error" role="alert">
          {problem}
        </p>
      )}
      {loaded.state === "loading" && <p className="vl-muted">{t("loading")}</p>}
      {loaded.state === "failed" && (
        <p className="vl-field-error" role="alert">
          {loaded.message}
        </p>
      )}
      {loaded.state === "ready" &&
        (loaded.value.length === 0 ? (
          <p className="vl-muted">{t("files-empty")}</p>
        ) : (
          <ul className="vl-files-list">
            {loaded.value.map((file) => (
              <FileRow
                key={file.name}
                file={file}
                canWrite={canWrite}
                onDelete={() => void remove(file)}
              />
            ))}
          </ul>
        ))}
    </>
  );
}

/** A page's attached files: download, upload and delete (issue #518, D277). */
export function FilesPanel({
  canWrite,
  onClose,
}: {
  readonly canWrite: boolean;
  readonly onClose: () => void;
}) {
  const t = useT();
  const headingId = useId();
  const files = usePageFiles();
  if (files === undefined) return;
  return (
    <aside className="vl-files-panel" aria-labelledby={headingId}>
      <div className="vl-files-head">
        <h2 id={headingId}>{t("files-title")}</h2>
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("files-close")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </div>
      <FileList files={files} canWrite={canWrite} />
    </aside>
  );
}
