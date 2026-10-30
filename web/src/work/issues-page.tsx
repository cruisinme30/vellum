// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { IssueOut, TeamOut, WorkApi } from "./api";

import "./work.css";

/** An issue's state as its team names it. */
export function stateName(team: TeamOut | undefined, state: string): string {
  return team?.statuses.find((status) => status.key === state)?.name ?? state;
}

function IssueList({
  issues,
  team,
}: {
  readonly issues: readonly IssueOut[];
  readonly team: TeamOut;
}) {
  const t = useT();
  if (issues.length === 0) return <p className="vl-muted">{t("issues-empty")}</p>;
  return (
    <ul className="vl-issue-list">
      {issues.map((issue) => (
        <li key={issue.id}>
          <Link to={`/issues/${encodeURIComponent(issue.key)}`}>
            <span className="vl-issue-key">{issue.key}</span>
            <span className="vl-issue-title">{issue.title}</span>
            <small className="vl-muted">{stateName(team, issue.state)}</small>
          </Link>
        </li>
      ))}
    </ul>
  );
}

function NewIssueForm({ api, team }: { readonly api: WorkApi; readonly team: TeamOut }) {
  const t = useT();
  const navigate = useNavigate();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const title = new FormData(event.currentTarget).get("title");
    if (typeof title !== "string" || title.trim() === "") return;
    setBusy(true);
    setError(undefined);
    api.createIssue({ team: team.key, title: title.trim() }).then(
      (issue) => {
        void navigate(`/issues/${encodeURIComponent(issue.key)}`);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  return (
    <form className="vl-work-add" onSubmit={submit}>
      <label>
        {t("issues-new", { team: team.name })}
        <input name="title" autoComplete="off" required />
      </label>
      <button type="submit" className="vl-filled-button" disabled={busy}>
        {t("issues-create")}
      </button>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

function TeamIssues({ api, team }: { readonly api: WorkApi; readonly team: TeamOut }) {
  const loaded = useLoaded(() => api.issues(team.key), [api, team.key], failureText);
  return (
    <>
      <NewIssueForm api={api} team={team} />
      <Ready loaded={loaded}>{(issues) => <IssueList issues={issues} team={team} />}</Ready>
    </>
  );
}

function Teams({ api, teams }: { readonly api: WorkApi; readonly teams: readonly TeamOut[] }) {
  const t = useT();
  const id = useId();
  const [parameters, setParameters] = useSearchParams();
  if (teams.length === 0) return <p className="vl-muted">{t("issues-no-teams")}</p>;
  const asked = parameters.get("team");
  const team = teams.find((each) => each.key === asked) ?? teams[0];
  if (team === undefined) return;
  return (
    <>
      <div className="vl-work-filters">
        <label htmlFor={id}>{t("issues-team")}</label>
        <select
          id={id}
          value={team.key}
          onChange={(event) => {
            setParameters({ team: event.target.value }, { replace: true });
          }}
        >
          {teams.map((each) => (
            <option key={each.key} value={each.key}>
              {each.name}
            </option>
          ))}
        </select>
      </div>
      <TeamIssues key={team.key} api={api} team={team} />
    </>
  );
}

function IssuesForApi({ api }: { readonly api: WorkApi }) {
  const loaded = useLoaded(() => api.teams(), [api], failureText);
  return <Ready loaded={loaded}>{(teams) => <Teams api={api} teams={teams} />}</Ready>;
}

/** Issues: a team's open issues, and a form to file a new one. */
export function IssuesPage() {
  const t = useT();
  return (
    <ServerScreen api="work" help="work.create-issue" title={t("issues")} className="vl-work">
      {(api) => <IssuesForApi api={api} />}
    </ServerScreen>
  );
}
