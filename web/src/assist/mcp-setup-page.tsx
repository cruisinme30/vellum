// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps } from "react";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { ConnectionReport, McpSetupApi, Snippet } from "./api";

import "../admin/console/console.css";
import "./assist.css";

/**
 * `/admin/mcp`: setup snippets for MCP clients and "Test connection" (R-AI-5).
 */
export function McpSetupPage() {
  const t = useT();
  return (
    <ServerScreen api="mcpSetup" title={t("admin-mcp")} className="vl-admin">
      {(api) => <McpSetup api={api} />}
    </ServerScreen>
  );
}

function CopyButton({
  text,
  label,
}: {
  readonly text: string;
  readonly label: string;
}) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const clipboard = globalThis.navigator.clipboard as Clipboard | undefined;
  if (clipboard === undefined) return;
  return (
    <button
      type="button"
      className="vl-text-button"
      aria-label={label}
      onClick={() => {
        void clipboard.writeText(text).then(() => {
          setCopied(true);
        });
      }}
    >
      {copied ? t("admin-mcp-copied") : t("admin-mcp-copy")}
    </button>
  );
}

function SnippetCard({ snippet }: { readonly snippet: Snippet }) {
  const t = useT();
  return (
    <li className="vl-mcp-snippet">
      <h2>{snippet.title}</h2>
      <p>{snippet.instructions}</p>
      {/* Focusable, so a keyboard can scroll a long command (WCAG 2.1.1). */}
      <pre
        // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex
        tabIndex={0}
        aria-label={t("admin-mcp-code", { client: snippet.title })}
      >
        <code>{snippet.code}</code>
      </pre>
      <div>
        <CopyButton
          text={snippet.code}
          label={t("admin-mcp-copy-for", { client: snippet.title })}
        />
      </div>
    </li>
  );
}

function TestConnection({ api }: { readonly api: McpSetupApi }) {
  const t = useT();
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<ConnectionReport>();
  const [error, setError] = useState<string>();
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    setBusy(true);
    setReport(undefined);
    setError(undefined);
    api
      .testConnection(token)
      .then(setReport, (error_: unknown) => {
        setError(failureText(error_));
      })
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <section aria-labelledby="mcp-test-heading">
      <h2 id="mcp-test-heading">{t("admin-mcp-test")}</h2>
      <p className="vl-muted">{t("admin-mcp-test-intro")}</p>
      <form className="vl-admin-form" onSubmit={submit}>
        <label>
          {t("admin-mcp-token")}
          <input
            type="password"
            value={token}
            required
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => {
              setToken(event.target.value);
            }}
          />
        </label>
        <div className="vl-admin-actions">
          <button
            type="submit"
            className="vl-filled-button"
            disabled={busy || token.trim() === ""}
          >
            {busy ? t("admin-mcp-testing") : t("admin-mcp-test")}
          </button>
        </div>
      </form>
      <div role="status">
        {report !== undefined && (
          <>
            <p>{report.ok ? t("admin-mcp-ok") : t("admin-mcp-failed")}</p>
            <ul className="vl-mcp-checks">
              {report.checks.map((check) => (
                <li key={check.name}>
                  {check.ok ? "✓" : "✗"} {check.detail}
                </li>
              ))}
            </ul>
            {report.scopes.length > 0 && (
              <p className="vl-muted">
                {t("admin-mcp-scopes", { scopes: report.scopes.join(", ") })}
              </p>
            )}
          </>
        )}
      </div>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

/** The page's body; exported for tests. */
export function McpSetup({ api }: { readonly api: McpSetupApi }) {
  const t = useT();
  const snippets = useLoaded(() => api.snippets(), [api], failureText);
  return (
    <>
      <p className="vl-muted">{t("admin-mcp-intro")}</p>
      <Ready loaded={snippets}>
        {(list) => (
          <ul className="vl-mcp-snippets">
            {list.map((snippet) => (
              <SnippetCard key={snippet.client} snippet={snippet} />
            ))}
          </ul>
        )}
      </Ready>
      {/* Below the snippets once they are in, so it does not jump down. */}
      {snippets.state !== "loading" && <TestConnection api={api} />}
    </>
  );
}
