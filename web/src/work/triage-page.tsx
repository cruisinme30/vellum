// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The triage inbox (R-WK-9): issues waiting for a team member, oldest
 * first, each with its SLA clock, and the four decisions: accept into a
 * status, decline with a reason, close as a duplicate, or snooze.
 */

import { useId, useState, type ComponentProps } from "react";
import { Link } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { IssueOut, TeamOut, TriageDecision, WorkApi } from "./api";
import { IssueChips } from "./chips";
import { fieldText } from "./form";
import { TeamPicker, useTeam, useWorkKeys, WorkNav } from "./nav";

import "./work.css";

type Mode = "none" | "decline" | "duplicate" | "snooze";

const SNOOZES = [
  ["work-snooze-day", 1],
  ["work-snooze-3-days", 3],
  ["work-snooze-week", 7],
] as const;

/** `days` from `from`, as RFC 3339. */
export function snoozeUntil(days: number, from: Date = new Date()): string {
  return new Date(from.getTime() + days * 86_400_000).toISOString();
}

function Decide({
  api,
  team,
  issue,
  onDone,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly issue: IssueOut;
  readonly onDone: (message: string) => void;
}) {
  const t = useT();
  const id = useId();
  const [mode, setMode] = useState<Mode>("none");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const accepting = team.statuses.filter(
    (status) =>
      status.category !== "triage" &&
      status.category !== "canceled" &&
      status.category !== "duplicate" &&
      status.category !== "completed",
  );
  const [state, setState] = useState(
    accepting.some((status) => status.key === team.default_status)
      ? team.default_status
      : (accepting[0]?.key ?? ""),
  );
  const decide = (decision: TriageDecision, message: string) => {
    setBusy(true);
    setError(undefined);
    api.triage(issue.key, decision).then(
      () => {
        onDone(message);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    switch (mode) {
      case "decline": {
        const reason = fieldText(data, "reason");
        decide(
          { action: "decline", ...(reason === "" ? {} : { reason }) },
          t("work-triage-declined", { key: issue.key }),
        );

        break;
      }
      case "duplicate": {
        const of = fieldText(data, "of").toUpperCase();
        if (of !== "")
          decide(
            { action: "duplicate", duplicate_of: of },
            t("work-triage-duplicated", { key: issue.key, of }),
          );

        break;
      }
      case "snooze": {
        const days = Number(fieldText(data, "days") || "1");
        decide(
          { action: "snooze", until: snoozeUntil(days) },
          t("work-triage-snoozed", { key: issue.key }),
        );

        break;
      }
      // No default
    }
  };
  return (
    <div className="vl-triage-actions">
      <span className="vl-work-field">
        <label htmlFor={`${id}-state`}>{t("work-triage-into")}</label>
        <select
          id={`${id}-state`}
          value={state}
          onChange={(event) => {
            setState(event.target.value);
          }}
        >
          {accepting.map((status) => (
            <option key={status.key} value={status.key}>
              {status.name}
            </option>
          ))}
        </select>
      </span>
      <button
        type="button"
        className="vl-filled-button"
        disabled={busy}
        onClick={() => {
          decide(
            { action: "accept", ...(state === "" ? {} : { state }) },
            t("work-triage-accepted", { key: issue.key }),
          );
        }}
      >
        {t("work-triage-accept")}
      </button>
      {(["decline", "duplicate", "snooze"] as const).map((each) => (
        <button
          key={each}
          type="button"
          className="vl-text-button"
          aria-expanded={mode === each}
          disabled={busy}
          onClick={() => {
            setMode(mode === each ? "none" : each);
          }}
        >
          {t(`work-triage-${each}`)}
        </button>
      ))}
      {mode !== "none" && (
        <form className="vl-work-add" onSubmit={submit}>
          {mode === "decline" && (
            <label>
              {t("work-triage-reason")}
              <input name="reason" autoComplete="off" />
            </label>
          )}
          {mode === "duplicate" && (
            <label>
              {t("work-triage-of")}
              <input name="of" autoComplete="off" required placeholder={`${team.key}-1`} />
            </label>
          )}
          {mode === "snooze" && (
            <label>
              {t("work-triage-until")}
              <select name="days" defaultValue="1">
                {SNOOZES.map(([label, days]) => (
                  <option key={days} value={days}>
                    {t(label)}
                  </option>
                ))}
              </select>
            </label>
          )}
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t(`work-triage-confirm-${mode}`)}
          </button>
        </form>
      )}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function Inbox({ api, team }: { readonly api: WorkApi; readonly team: TeamOut }) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [announce, setAnnounce] = useState("");
  useWorkKeys(team.key);
  const loaded = useLoaded(() => api.inbox(team.key), [api, team.key, version], failureText);
  if (!team.triage) {
    return <p className="vl-muted">{t("work-triage-off", { team: team.name })}</p>;
  }
  return (
    <>
      <p className="vl-visually-hidden" role="status" aria-live="polite">
        {announce}
      </p>
      <Ready loaded={loaded}>
        {(issues) =>
          issues.length === 0 ? (
            <p className="vl-muted">{t("work-triage-empty")}</p>
          ) : (
            <ul className="vl-triage-list">
              {issues.map((issue) => (
                <li key={issue.id} className="vl-triage-item">
                  <Link to={`/issues/${encodeURIComponent(issue.key)}`} data-issue={issue.key}>
                    <span className="vl-issue-key">{issue.key}</span>
                    <span className="vl-issue-title">{issue.title}</span>
                    <IssueChips issue={issue} />
                  </Link>
                  {issue.creator !== undefined && issue.creator !== null && (
                    <p className="vl-muted">{t("work-triage-from", { person: issue.creator })}</p>
                  )}
                  <Decide
                    api={api}
                    team={team}
                    issue={issue}
                    onDone={(message) => {
                      setAnnounce(message);
                      setVersion((v) => v + 1);
                    }}
                  />
                </li>
              ))}
            </ul>
          )
        }
      </Ready>
    </>
  );
}

function TriageForTeams({
  api,
  teams,
}: {
  readonly api: WorkApi;
  readonly teams: readonly TeamOut[];
}) {
  const t = useT();
  const team = useTeam(teams);
  if (team === undefined) return <p className="vl-muted">{t("issues-no-teams")}</p>;
  return (
    <>
      <div className="vl-work-filters">
        <TeamPicker teams={teams} team={team} />
      </div>
      <Inbox key={team.key} api={api} team={team} />
    </>
  );
}

function TriageLoader({ api }: { readonly api: WorkApi }) {
  const loaded = useLoaded(() => api.teams(), [api], failureText);
  return <Ready loaded={loaded}>{(teams) => <TriageForTeams api={api} teams={teams} />}</Ready>;
}

/** The triage inbox screen. */
export function TriagePage() {
  const t = useT();
  return (
    <ServerScreen api="work" title={t("work-triage")} className="vl-work">
      {(api) => (
        <>
          <WorkNav />
          <TriageLoader api={api} />
        </>
      )}
    </ServerScreen>
  );
}
