// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../../data/failure";
import { useServerApi } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { PlanningApi } from "./api";
import { Failure, priorityId } from "./shared";

function Suggestion({
  api,
  issueKey,
  onApplied,
}: {
  readonly api: PlanningApi;
  readonly issueKey: string;
  readonly onApplied?: (() => void) | undefined;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(
    () => api.brief(issueKey),
    [api, issueKey, version],
    failureText,
  );
  if (loaded.state !== "ready") return;
  const s = loaded.value.suggestion;
  if (s == undefined) return;
  const act = (accept: boolean) => {
    setBusy(true);
    setError(undefined);
    (accept
      ? api.acceptSuggestion(issueKey)
      : api.dismissSuggestion(issueKey)
    ).then(
      () => {
        setBusy(false);
        setVersion((v) => v + 1);
        if (accept) onApplied?.();
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  const parts = [
    s.team != undefined && t("plan-suggest-team", { team: s.team }),
    s.priority != undefined &&
      t("plan-suggest-priority", {
        priority: t(priorityId(s.priority)),
      }),
    s.labels.length > 0 &&
      t("plan-suggest-labels", { labels: s.labels.join(", ") }),
    s.assignee != undefined &&
      t("plan-suggest-assignee", { assignee: `@${s.assignee}` }),
    s.duplicate_of != undefined &&
      t("plan-suggest-duplicate", { key: s.duplicate_of }),
  ].filter((p): p is string => typeof p === "string");
  return (
    <aside
      className="vl-plan-suggestion"
      aria-labelledby={`vl-suggest-${issueKey}`}
    >
      <h2 id={`vl-suggest-${issueKey}`}>
        {t("plan-suggest-title", {
          model: s.model === "" ? t("plan-suggest-assistant") : s.model,
        })}
      </h2>
      <ul>
        {parts.map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
      <p className="vl-muted">{s.reason}</p>
      <div className="vl-plan-actions">
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy}
          onClick={() => {
            act(true);
          }}
        >
          {t("plan-suggest-accept")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            act(false);
          }}
        >
          {t("plan-suggest-dismiss")}
        </button>
      </div>
      <Failure error={error} />
    </aside>
  );
}

/**
 * The triage decision an assistant proposed for an issue, if one is
 * waiting, with Accept and Dismiss (R-WK-16). Shows nothing otherwise, and
 * nothing without a server.
 */
export function TriageSuggestion({
  issueKey,
  onApplied,
}: {
  readonly issueKey: string;
  readonly onApplied?: () => void;
}) {
  const api = useServerApi("planning");
  if (api === undefined) return;
  return <Suggestion api={api} issueKey={issueKey} onApplied={onApplied} />;
}
