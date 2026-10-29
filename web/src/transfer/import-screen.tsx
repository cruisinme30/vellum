// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type SyntheticEvent } from "react";
import { Link } from "react-router";

import { ServerScreen } from "../data/server-apis";
import { useServices, useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { guessSource, type ImportResult, type ImportSource, type TransferApi } from "./api";

import "./transfer.css";

const SOURCES: readonly ImportSource[] = [
  "notion",
  "confluence",
  "docx",
  "markdown",
  "obsidian",
  "outline",
  "docmost",
];

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

function ImportForm({ api }: { readonly api: TransferApi }) {
  const t = useT();
  const ids = { source: useId(), space: useId(), folder: useId(), file: useId() };
  const workspace = useWorkspace();
  const { store } = useServices();
  const [source, setSource] = useState<ImportSource>("notion");
  const [space, setSpace] = useState(workspace.spaces[0]?.id ?? "");
  const [folder, setFolder] = useState("");
  const [file, setFile] = useState<File>();
  const [state, setState] = useState<
    | { readonly kind: "idle" }
    | { readonly kind: "working" }
    | { readonly kind: "done"; readonly result: ImportResult }
    | { readonly kind: "failed"; readonly message: string }
  >({ kind: "idle" });
  const spaceId = space === "" ? (workspace.spaces[0]?.id ?? "") : space;
  const folders = workspace.pages.filter((p) => p.spaceId === spaceId && p.folder === true);

  const submit = async (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (file === undefined || spaceId === "") return;
    setState({ kind: "working" });
    try {
      const result = await api.importFile(file, {
        space: spaceId,
        parent: folder === "" ? undefined : folder,
        source,
      });
      store.refresh?.();
      setState({ kind: "done", result });
    } catch (error) {
      setState({ kind: "failed", message: errorText(error) });
    }
  };

  return (
    <form className="vl-import-form" onSubmit={(event) => void submit(event)}>
      <p className="vl-muted">{t("import-intro")}</p>
      <label htmlFor={ids.file}>{t("import-file")}</label>
      <input
        id={ids.file}
        type="file"
        accept=".zip,.docx"
        required
        onChange={(event) => {
          const chosen = event.currentTarget.files?.[0];
          setFile(chosen);
          const guess = chosen === undefined ? undefined : guessSource(chosen.name);
          if (guess !== undefined) setSource(guess);
        }}
      />
      <label htmlFor={ids.source}>{t("import-source")}</label>
      <select
        id={ids.source}
        value={source}
        onChange={(event) => {
          setSource(event.currentTarget.value as ImportSource);
        }}
      >
        {SOURCES.map((value) => (
          <option key={value} value={value}>
            {t(`import-source-${value}`)}
          </option>
        ))}
      </select>
      <label htmlFor={ids.space}>{t("import-space")}</label>
      <select
        id={ids.space}
        value={spaceId}
        onChange={(event) => {
          setSpace(event.currentTarget.value);
          setFolder("");
        }}
      >
        {workspace.spaces.map((s) => (
          <option key={s.id} value={s.id}>
            {s.name}
          </option>
        ))}
      </select>
      <label htmlFor={ids.folder}>{t("import-folder")}</label>
      <select
        id={ids.folder}
        value={folder}
        onChange={(event) => {
          setFolder(event.currentTarget.value);
        }}
      >
        <option value="">{t("import-folder-top")}</option>
        {folders.map((f) => (
          <option key={f.id} value={f.id}>
            {f.title}
          </option>
        ))}
      </select>
      <div>
        <button
          type="submit"
          className="vl-filled-button"
          disabled={file === undefined || spaceId === "" || state.kind === "working"}
        >
          <Icon name="upload" size={20} />
          {state.kind === "working" ? t("import-working") : t("import-start")}
        </button>
      </div>
      <div role="status">
        {state.kind === "done" && (
          <div className="vl-import-done">
            <p>
              {t("import-done", { pages: state.result.nodes, title: state.result.title })}{" "}
              <Link to={`/p/${encodeURIComponent(state.result.folder_id)}`}>
                {t("import-open")}
              </Link>
            </p>
            {state.result.warnings.length > 0 && (
              <details className="vl-transfer-warnings">
                <summary>{t("export-warnings", { count: state.result.warnings.length })}</summary>
                <ul>
                  {state.result.warnings.map((w, index) => (
                    <li key={index}>
                      <strong>{w.location}</strong>: {w.message}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
      </div>
      {state.kind === "failed" && (
        <p className="vl-field-error" role="alert">
          {state.message}
        </p>
      )}
    </form>
  );
}

/** Import: a Notion, Confluence or Markdown export, or a Word file, into a new folder. */
export function ImportScreen() {
  const t = useT();
  return (
    <ServerScreen api="transfer" title={t("import-title")}>
      {(api) => <ImportForm api={api} />}
    </ServerScreen>
  );
}
