// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useRef, useState } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { uploadProblem, type Attachment } from "./api";
import { usePageFiles } from "./context";

/**
 * A button that picks a file and uploads it to the page on view, with a
 * live status line and a plain-words reason when the server refuses it.
 * Nothing shows when uploads are not available.
 */
export function UploadButton({
  label,
  accept,
  onUploaded,
}: {
  readonly label: string;
  /** File types the picker offers, such as `image/*`. */
  readonly accept?: string;
  readonly onUploaded: (file: Attachment) => void;
}) {
  const t = useT();
  const files = usePageFiles();
  const input = useRef<HTMLInputElement>(null);
  const inputId = useId();
  const [state, setState] = useState<
    | { kind: "idle" }
    | { kind: "busy"; name: string }
    | { kind: "failed"; message: string }
  >({ kind: "idle" });
  if (files === undefined) return;

  const upload = async (file: File) => {
    setState({ kind: "busy", name: file.name });
    try {
      const stored = await files.api.upload(files.pageId, file);
      setState({ kind: "idle" });
      onUploaded(stored);
    } catch (error) {
      const problem = uploadProblem(error);
      setState({
        kind: "failed",
        message: `${t(problem.key)} ${problem.detail}`,
      });
    }
  };

  return (
    <div className="vl-upload">
      <input
        ref={input}
        id={inputId}
        type="file"
        className="vl-visually-hidden"
        tabIndex={-1}
        aria-hidden="true"
        {...(accept === undefined ? {} : { accept })}
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file !== undefined) void upload(file);
        }}
      />
      <button
        type="button"
        className="vl-text-button"
        disabled={state.kind === "busy"}
        onClick={() => {
          input.current?.click();
        }}
      >
        <Icon name="upload" size={20} />
        {label}
      </button>
      <span role="status" className="vl-muted">
        {state.kind === "busy" ? t("upload-busy", { name: state.name }) : ""}
      </span>
      {state.kind === "failed" && (
        <span className="vl-field-error" role="alert">
          {state.message}
        </span>
      )}
    </div>
  );
}
