// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A team's open repeating issues, with their repeat changed or stopped (#144).
 */

import { useId, useState } from "react";
import { Link } from "react-router";

import { failureText } from "../../data/failure";
import { Ready } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import type { TeamOut } from "../api";
import type { IssueOut, PlanningApi } from "../planning/api";

import { repeatChoices } from "./template-file";

/** One repeating issue, with its repeat changed or stopped in place. */
function RecurringRow({
  planning,
  issue,
  onChanged,
}: {
  readonly planning: PlanningApi;
  readonly issue: IssueOut;
  readonly onChanged: (message: string) => void;
}) {
  const t = useT();
  const id = useId();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const repeat = issue.repeat ?? "";
  const change = (next: string, message: string) => {
    setBusy(true);
    setError(undefined);
    planning.setRepeat(issue.key, next).then(
      () => {
        setBusy(false);
        onChanged(message);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  return (
    <li className="vl-tpl-recurring">
      <span className="vl-tpl-recurring-title">
        <Link to={`/issues/${encodeURIComponent(issue.key)}`}>{issue.key}</Link>{" "}
        {issue.title}
      </span>
      <span className="vl-muted">
        {issue.due == undefined
          ? t("tpl-no-due")
          : t("tpl-due", { due: issue.due })}
      </span>
      <label className="vl-visually-hidden" htmlFor={id}>
        {t("tpl-repeat-of", { key: issue.key })}
      </label>
      <select
        id={id}
        value={repeat}
        disabled={busy}
        onChange={(event) => {
          const next = event.target.value;
          change(
            next,
            t("tpl-repeat-changed", { key: issue.key, repeat: next }),
          );
        }}
      >
        {repeatChoices(repeat).map((r) => (
          <option key={r} value={r}>
            {t("plan-repeat-every", { repeat: r })}
          </option>
        ))}
      </select>
      <button
        type="button"
        className="vl-text-button"
        disabled={busy}
        aria-label={t("tpl-stop-label", { key: issue.key })}
        onClick={() => {
          change("", t("tpl-repeat-stopped", { key: issue.key }));
        }}
      >
        {t("tpl-stop")}
      </button>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </li>
  );
}

/** The recurring issues section. */
export function Recurring({
  planning,
  team,
  version,
  onChanged,
}: {
  readonly planning: PlanningApi;
  readonly team: TeamOut;
  readonly version: number;
  readonly onChanged: (message: string) => void;
}) {
  const t = useT();
  const loaded = useLoaded(
    () => planning.recurring(team.key),
    [planning, team.key, version],
    failureText,
  );
  return (
    <section className="vl-settings-section" aria-labelledby="vl-tpl-recurring">
      <h2 id="vl-tpl-recurring">{t("tpl-recurring")}</h2>
      <p className="vl-muted">
        {t("tpl-recurring-intro", { timezone: team.timezone })}
      </p>
      <Ready loaded={loaded}>
        {(issues) =>
          issues.length === 0 ? (
            <p className="vl-muted">{t("tpl-recurring-empty")}</p>
          ) : (
            <ul className="vl-tpl-list">
              {issues.map((issue) => (
                <RecurringRow
                  key={`${issue.key}:${issue.repeat ?? ""}`}
                  planning={planning}
                  issue={issue}
                  onChanged={onChanged}
                />
              ))}
            </ul>
          )
        }
      </Ready>
    </section>
  );
}
