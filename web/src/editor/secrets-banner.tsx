// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The warning banner for secrets in a page (R-SE-6, D296). The server
 * scans each save under the workspace's policy; this asks it what a save
 * of the page as it stands would meet, and names each likely secret's kind
 * and line, with Remove and It's not a secret.
 */

import type { Node as ProseNode } from "@tiptap/pm/model";
import type { Editor } from "@tiptap/react";
import { useEffect, useState } from "react";

import type { SecretFinding, SecretScanResult } from "../api/api.gen";
import { useT } from "../i18n/i18n";

/** Asks the server about some page text. */
export type ScanSecrets = (text: string) => Promise<SecretScanResult>;

/** How long after the last change the page is scanned again. */
const SCAN_DELAY_MS = 1500;

/** The page's text, one line per text block, and where each block starts. */
export function pageText(document_: ProseNode): {
  text: string;
  starts: number[];
} {
  const lines: string[] = [];
  const starts: number[] = [];
  document_.descendants((node, pos) => {
    if (!node.isTextblock) return true;
    lines.push(node.textContent);
    // The block's content starts one position in.
    starts.push(pos + 1);
    return false;
  });
  return { text: lines.join("\n"), starts };
}

/** A finding's range in the document, when its text is still there. */
function findingRange(
  document_: ProseNode,
  starts: readonly number[],
  finding: SecretFinding,
): { from: number; to: number } | undefined {
  const start = starts[finding.line - 1];
  if (start === undefined || finding.length <= 0) return undefined;
  const from = start + finding.column - 1;
  const to = from + finding.length;
  if (to > document_.content.size) return undefined;
  const head = finding.redacted.split("…")[0] ?? "";
  const found = document_.textBetween(from, to);
  return found.length === finding.length && found.startsWith(head)
    ? { from, to }
    : undefined;
}

/** A finding's identity, to remember the ones dismissed. */
const keyOf = (finding: SecretFinding) => `${finding.rule}:${finding.redacted}`;

/** The banner; nothing while the page holds no likely secret. */
export function SecretsBanner({
  editor,
  scan,
}: {
  readonly editor: Editor;
  readonly scan: ScanSecrets;
}) {
  const t = useT();
  const [result, setResult] = useState<{
    scan: SecretScanResult;
    starts: number[];
  }>();
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let current = true;
    const run = () => {
      const { text, starts } = pageText(editor.state.doc);
      scan(text).then(
        (found) => {
          if (current) setResult({ scan: found, starts });
        },
        () => {
          // The server enforces the policy on save; a failed check only
          // means no banner.
        },
      );
    };
    const later = () => {
      clearTimeout(timer);
      timer = setTimeout(run, SCAN_DELAY_MS);
    };
    run();
    editor.on("update", later);
    return () => {
      current = false;
      clearTimeout(timer);
      editor.off("update", later);
    };
  }, [editor, scan]);

  const shown = (result?.scan.findings ?? []).filter(
    (f) => !dismissed.has(keyOf(f)),
  );
  if (
    result === undefined ||
    shown.length === 0 ||
    result.scan.policy === "off"
  )
    return;
  const blocked = result.scan.policy === "block";
  return (
    <div
      className="vl-secrets-banner"
      role="alert"
      data-policy={result.scan.policy}
    >
      <p>{t(blocked ? "secrets-blocked" : "secrets-warning")}</p>
      <ul>
        {shown.map((finding) => {
          const range = findingRange(editor.state.doc, result.starts, finding);
          return (
            <li key={keyOf(finding)}>
              <span>
                {t("secrets-finding", {
                  kind: t(`secret-kind-${finding.rule}`),
                  line: finding.line,
                  preview: finding.redacted,
                })}
              </span>
              {range && editor.isEditable && (
                <button
                  type="button"
                  className="vl-text-button"
                  onClick={() => {
                    editor.chain().focus().deleteRange(range).run();
                  }}
                >
                  {t("secrets-remove")}
                </button>
              )}
              {!blocked && (
                <button
                  type="button"
                  className="vl-text-button"
                  onClick={() => {
                    setDismissed(new Set([...dismissed, keyOf(finding)]));
                  }}
                >
                  {t("secrets-not-a-secret")}
                </button>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
