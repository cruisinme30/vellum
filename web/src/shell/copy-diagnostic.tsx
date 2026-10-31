// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import {
  diagnosticText,
  recentDiagnostic,
  type Diagnostic,
} from "../data/failure";
import { useT } from "../i18n/i18n";

import { Icon } from "./icon";

/**
 * "Copy diagnostic" for an error message (R-OB-1): copies the request ID,
 * route and time so a person can hand them to whoever runs the server.
 * Give it the failure line from `failureText`, or the diagnostic itself.
 * Where the clipboard is blocked, the text is shown to copy by hand.
 */
export function CopyDiagnostic({
  message,
  diagnostic,
}: {
  readonly message?: string | undefined;
  readonly diagnostic?: Diagnostic | undefined;
}) {
  const t = useT();
  const [state, setState] = useState<"idle" | "copied" | { manual: string }>(
    "idle",
  );
  const found =
    diagnostic ??
    (message === undefined
      ? undefined
      : (recentDiagnostic(message) ?? { message, at: new Date() }));
  if (found === undefined) return;
  const copy = async () => {
    const text = diagnosticText(found, globalThis.location.pathname);
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState({ manual: text });
    }
  };
  return (
    <span className="vl-diagnostic">
      <button
        type="button"
        className="vl-text-button vl-diagnostic-copy"
        onClick={() => {
          void copy();
        }}
      >
        <Icon name="content_copy" />
        {t("diagnostic-copy")}
      </button>
      <span role="status" className="vl-diagnostic-status">
        {state === "copied" ? t("diagnostic-copied") : ""}
      </span>
      {typeof state === "object" && (
        <textarea
          className="vl-diagnostic-text"
          aria-label={t("diagnostic-manual")}
          readOnly
          rows={6}
          value={state.manual}
          ref={(node) => {
            node?.select();
          }}
        />
      )}
    </span>
  );
}
