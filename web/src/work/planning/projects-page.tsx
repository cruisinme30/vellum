// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useNavigate } from "react-router";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { PlanningApi, ProjectOut, TeamOut } from "./api";
import {
  Failure,
  Field,
  HealthChip,
  Progress,
  day,
  text,
  useSubmit,
} from "./shared";

const STATUSES = [
  "backlog",
  "planned",
  "in-progress",
  "completed",
  "canceled",
] as const;

function NewProject({
  api,
  teams,
}: {
  readonly api: PlanningApi;
  readonly teams: readonly TeamOut[];
}) {
  const t = useT();
  const navigate = useNavigate();
  const { busy, error, onSubmit } = useSubmit(
    (form) => {
      const title = text(form, "title");
      if (title === "") return;
      return api.createProject({
        title,
        teams: [text(form, "team")],
        lead: text(form, "lead"),
        status: text(form, "status"),
        start_date: text(form, "start"),
        target_date: text(form, "target"),
      });
    },
    (project) => {
      void navigate(`/projects/${encodeURIComponent(project.id)}`);
    },
  );
  if (teams.length === 0)
    return <p className="vl-muted">{t("issues-no-teams")}</p>;
  return (
    <details className="vl-plan-new">
      <summary>{t("plan-project-new")}</summary>
      <form className="vl-plan-form" onSubmit={onSubmit}>
        <Field label={t("plan-title")}>
          <input name="title" required maxLength={200} autoComplete="off" />
        </Field>
        <Field label={t("issues-team")}>
          <select name="team">
            {teams.map((team) => (
              <option key={team.key} value={team.key}>
                {team.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("plan-lead")}>
          <input name="lead" placeholder="@handle" autoComplete="off" />
        </Field>
        <Field label={t("plan-status")}>
          <select name="status" defaultValue="planned">
            {STATUSES.map((status) => (
              <option key={status} value={status}>
                {t(`plan-status-${status}`)}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("plan-start")}>
          <input name="start" type="date" />
        </Field>
        <Field label={t("plan-target")}>
          <input name="target" type="date" />
        </Field>
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("plan-project-create")}
        </button>
        <Failure error={error} />
      </form>
    </details>
  );
}

/** One project as a row: health, progress, lead and target date. */
export function ProjectRow({ project }: { readonly project: ProjectOut }) {
  const t = useT();
  return (
    <li className="vl-plan-row">
      <Link to={`/projects/${encodeURIComponent(project.id)}`}>
        <span className="vl-plan-row-title">{project.title}</span>
        <HealthChip health={project.health} />
        <Progress
          percent={project.percent}
          label={t("plan-progress-of", { title: project.title })}
        />
        <small className="vl-muted">
          {t(`plan-status-${project.status}`)}
          {project.lead != undefined && ` · @${project.lead}`}
          {project.target_date != undefined &&
            ` · ${t("plan-due", { date: day(project.target_date) })}`}
          {project.update_due && ` · ${t("plan-update-due")}`}
        </small>
      </Link>
    </li>
  );
}

/** Whether a project passes the status filter (`active`, `all` or a status). */
export function inStatus(project: ProjectOut, status: string): boolean {
  if (status === "all") return true;
  if (status === "active")
    return project.status === "planned" || project.status === "in-progress";
  return project.status === status;
}

function Projects({
  api,
  projects,
  teams,
}: {
  readonly api: PlanningApi;
  readonly projects: readonly ProjectOut[];
  readonly teams: readonly TeamOut[];
}) {
  const t = useT();
  const [status, setStatus] = useState("active");
  const shown = projects
    .filter((p) => inStatus(p, status))
    .toSorted(
      (a, b) =>
        (a.target_date ?? "9999").localeCompare(b.target_date ?? "9999") ||
        a.title.localeCompare(b.title),
    );
  return (
    <>
      <NewProject api={api} teams={teams} />
      <div className="vl-work-filters">
        <label htmlFor="vl-plan-status">{t("plan-show")}</label>
        <select
          id="vl-plan-status"
          value={status}
          onChange={(event) => {
            setStatus(event.target.value);
          }}
        >
          <option value="active">{t("plan-status-active")}</option>
          {STATUSES.map((s) => (
            <option key={s} value={s}>
              {t(`plan-status-${s}`)}
            </option>
          ))}
          <option value="all">{t("plan-status-all")}</option>
        </select>
        <Link to="/initiatives">{t("plan-initiatives")}</Link>
        <Link to="/insights">{t("plan-insights")}</Link>
      </div>
      {shown.length === 0 ? (
        <p className="vl-muted">{t("plan-projects-empty")}</p>
      ) : (
        <ul className="vl-plan-list">
          {shown.map((project) => (
            <ProjectRow key={project.id} project={project} />
          ))}
        </ul>
      )}
    </>
  );
}

function ProjectsForApi({ api }: { readonly api: PlanningApi }) {
  const loaded = useLoaded(
    async () => {
      const [projects, teams] = await Promise.all([
        api.projects(),
        api.teams(),
      ]);
      return { projects, teams };
    },
    [api],
    failureText,
  );
  return (
    <Ready loaded={loaded}>
      {(found) => <Projects api={api} {...found} />}
    </Ready>
  );
}

/**
 * Projects: every project with health and progress, and a form for a new one.
 */
export function ProjectsPage() {
  const t = useT();
  return (
    <ServerScreen
      api="planning"
      title={t("plan-projects")}
      className="vl-work vl-plan"
    >
      {(api) => <ProjectsForApi api={api} />}
    </ServerScreen>
  );
}
