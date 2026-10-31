// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useParams } from "react-router";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { IssueOut, PlanningApi, ProjectOut, UpdateOut } from "./api";
import {
  Failure,
  Field,
  HEALTHS,
  HealthChip,
  Progress,
  day,
  text,
  useSubmit,
} from "./shared";

const WHEN = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

function Milestones({
  api,
  project,
  onChange,
}: {
  readonly api: PlanningApi;
  readonly project: ProjectOut;
  readonly onChange: () => void;
}) {
  const t = useT();
  const { busy, error, onSubmit } = useSubmit((form) => {
    const name = text(form, "name");
    if (name === "") return;
    return api.addMilestone(project.id, name, text(form, "target"));
  }, onChange);
  return (
    <section aria-labelledby="vl-plan-milestones">
      <h2 id="vl-plan-milestones">{t("plan-milestones")}</h2>
      {project.milestones.length === 0 ? (
        <p className="vl-muted">{t("plan-milestones-empty")}</p>
      ) : (
        <ol className="vl-plan-milestones">
          {project.milestones.map((m) => (
            <li key={m.id}>
              <span className="vl-plan-row-title">{m.name}</span>
              <Progress
                percent={m.percent}
                label={t("plan-progress-of", { title: m.name })}
              />
              <small className="vl-muted">
                {t("plan-issues-done", {
                  done: m.completed_issues,
                  total: m.issues,
                })}
                {m.target_date != undefined &&
                  ` · ${t("plan-due", { date: day(m.target_date) })}`}
              </small>
            </li>
          ))}
        </ol>
      )}
      <form className="vl-plan-form" onSubmit={onSubmit}>
        <Field label={t("plan-milestone-name")}>
          <input name="name" required maxLength={200} autoComplete="off" />
        </Field>
        <Field label={t("plan-target")}>
          <input name="target" type="date" />
        </Field>
        <button type="submit" className="vl-text-button" disabled={busy}>
          {t("plan-milestone-add")}
        </button>
        <Failure error={error} />
      </form>
    </section>
  );
}

function Updates({
  api,
  project,
  onChange,
}: {
  readonly api: PlanningApi;
  readonly project: ProjectOut;
  readonly onChange: () => void;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const loaded = useLoaded(
    () => api.updates(project.id),
    [api, project.id, version],
    failureText,
  );
  const { busy, error, onSubmit } = useSubmit(
    (form) =>
      api.postUpdate(project.id, text(form, "health"), text(form, "body")),
    () => {
      setVersion((v) => v + 1);
      onChange();
    },
  );
  return (
    <section aria-labelledby="vl-plan-updates">
      <h2 id="vl-plan-updates">{t("plan-updates")}</h2>
      {project.update_due && (
        <p className="vl-plan-due">{t("plan-update-due-long")}</p>
      )}
      <form className="vl-plan-form vl-plan-update-form" onSubmit={onSubmit}>
        <fieldset className="vl-plan-health-pick">
          <legend>{t("plan-health")}</legend>
          {HEALTHS.map((health, index) => (
            <label key={health}>
              <input
                type="radio"
                name="health"
                value={health}
                defaultChecked={index === 0}
              />
              <HealthChip health={health} />
            </label>
          ))}
        </fieldset>
        <Field label={t("plan-update-body")}>
          <textarea name="body" rows={3} maxLength={65_536} />
        </Field>
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("plan-update-post")}
        </button>
        <Failure error={error} />
      </form>
      <Ready loaded={loaded}>
        {(updates: UpdateOut[]) =>
          updates.length === 0 ? (
            <p className="vl-muted">{t("plan-updates-empty")}</p>
          ) : (
            <ol className="vl-plan-updates">
              {updates.map((u) => (
                <li key={u.id}>
                  <p>
                    <HealthChip health={u.health} />{" "}
                    <small className="vl-muted">
                      {t("issue-comment-by", {
                        author: `@${u.author}`,
                        when: WHEN.format(new Date(u.created_at)),
                      })}
                    </small>
                  </p>
                  {u.body.trim() !== "" && (
                    <p className="vl-issue-description">{u.body.trim()}</p>
                  )}
                  {u.changes.length > 0 && (
                    <ul className="vl-plan-changes">
                      {u.changes.map((c) => (
                        <li key={c}>{c}</li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ol>
          )
        }
      </Ready>
    </section>
  );
}

function ProjectIssues({
  api,
  project,
  onChange,
}: {
  readonly api: PlanningApi;
  readonly project: ProjectOut;
  readonly onChange: () => void;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(
    () => api.projectIssues(project.id),
    [api, project.id, version],
    failureText,
  );
  const team = project.teams[0] ?? "";
  const move = (issue: IssueOut, milestone: string) => {
    setError(undefined);
    api.setMilestone(issue.key, milestone).then(
      () => {
        setVersion((v) => v + 1);
        onChange();
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  return (
    <section aria-labelledby="vl-plan-issues">
      <h2 id="vl-plan-issues">{t("issues")}</h2>
      <p>
        <Link
          to={
            `/issues/new?team=${encodeURIComponent(team)}` +
            `&project=${encodeURIComponent(project.id)}`
          }
        >
          {t("plan-issue-new")}
        </Link>
      </p>
      <Failure error={error} />
      <Ready loaded={loaded}>
        {(issues: IssueOut[]) =>
          issues.length === 0 ? (
            <p className="vl-muted">{t("plan-project-no-issues")}</p>
          ) : (
            <ul className="vl-plan-issues">
              {issues.map((issue) => (
                <li key={issue.id}>
                  <Link to={`/issues/${encodeURIComponent(issue.key)}`}>
                    <span className="vl-issue-key">{issue.key}</span>{" "}
                    <span className="vl-issue-title">{issue.title}</span>
                  </Link>
                  <small className="vl-muted">{issue.state}</small>
                  {project.milestones.length > 0 && (
                    <select
                      aria-label={t("plan-milestone-of", { key: issue.key })}
                      value={issue.milestone ?? ""}
                      onChange={(event) => {
                        move(issue, event.target.value);
                      }}
                    >
                      <option value="">{t("plan-milestone-none")}</option>
                      {project.milestones.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.name}
                        </option>
                      ))}
                    </select>
                  )}
                </li>
              ))}
            </ul>
          )
        }
      </Ready>
    </section>
  );
}

function Project({
  api,
  projectId,
}: {
  readonly api: PlanningApi;
  readonly projectId: string;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const loaded = useLoaded(
    () => api.project(projectId),
    [api, projectId, version],
    failureText,
  );
  const refresh = () => {
    setVersion((v) => v + 1);
  };
  return (
    <Ready loaded={loaded}>
      {(project) => (
        <>
          <p className="vl-muted">
            <Link to="/projects">{t("plan-projects")}</Link> ·{" "}
            {t(`plan-status-${project.status}`)}
          </p>
          <h1>{project.title}</h1>
          <div className="vl-plan-summary">
            <HealthChip health={project.health} />
            <Progress
              percent={project.percent}
              label={t("plan-progress-of", { title: project.title })}
            />
            <span>
              {t("plan-issues-done", {
                done: project.completed_issues,
                total: project.issues,
              })}
            </span>
            {project.lead != undefined && (
              <span>{t("plan-lead-is", { lead: `@${project.lead}` })}</span>
            )}
            {project.start_date != undefined && (
              <span>{t("plan-starts", { date: day(project.start_date) })}</span>
            )}
            {project.target_date != undefined && (
              <span>{t("plan-due", { date: day(project.target_date) })}</span>
            )}
            <span>{project.teams.join(", ")}</span>
          </div>
          <Milestones api={api} project={project} onChange={refresh} />
          <Updates api={api} project={project} onChange={refresh} />
          <ProjectIssues api={api} project={project} onChange={refresh} />
        </>
      )}
    </Ready>
  );
}

/** One project: progress, milestones, health updates and its issues. */
export function ProjectPage() {
  const { projectId = "" } = useParams();
  return (
    <ServerScreen api="planning" className="vl-work vl-plan">
      {(api) => <Project api={api} projectId={projectId} />}
    </ServerScreen>
  );
}
