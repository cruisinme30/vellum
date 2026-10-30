// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link } from "react-router";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { InitiativeOut, PlanningApi, ProjectOut } from "./api";
import { ProjectRow } from "./projects-page";
import {
  Failure,
  Field,
  HealthChip,
  Progress,
  day,
  text,
  useSubmit,
} from "./shared";

function NewInitiative({
  api,
  projects,
  onCreated,
}: {
  readonly api: PlanningApi;
  readonly projects: readonly ProjectOut[];
  readonly onCreated: () => void;
}) {
  const t = useT();
  const { busy, error, onSubmit } = useSubmit((form) => {
    const title = text(form, "title");
    if (title === "") return;
    return api.createInitiative({
      title,
      owner: text(form, "owner"),
      target_date: text(form, "target"),
      projects: form
        .getAll("projects")
        .filter((p): p is string => typeof p === "string"),
    });
  }, onCreated);
  return (
    <details className="vl-plan-new">
      <summary>{t("plan-initiative-new")}</summary>
      <form className="vl-plan-form" onSubmit={onSubmit}>
        <Field label={t("plan-title")}>
          <input name="title" required maxLength={200} autoComplete="off" />
        </Field>
        <Field label={t("plan-owner")}>
          <input name="owner" placeholder="@handle" autoComplete="off" />
        </Field>
        <Field label={t("plan-target")}>
          <input name="target" type="date" />
        </Field>
        {projects.length > 0 && (
          <fieldset className="vl-plan-checks">
            <legend>{t("plan-projects")}</legend>
            {projects.map((p) => (
              <label key={p.id}>
                <input type="checkbox" name="projects" value={p.id} /> {p.title}
              </label>
            ))}
          </fieldset>
        )}
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("plan-initiative-create")}
        </button>
        <Failure error={error} />
      </form>
    </details>
  );
}

function Initiative({ initiative }: { readonly initiative: InitiativeOut }) {
  const t = useT();
  return (
    <li className="vl-plan-initiative">
      <h2>{initiative.title}</h2>
      <div className="vl-plan-summary">
        <HealthChip health={initiative.health} />
        <Progress
          percent={initiative.percent}
          label={t("plan-progress-of", { title: initiative.title })}
        />
        {initiative.owner != undefined && (
          <span>{t("plan-owner-is", { owner: `@${initiative.owner}` })}</span>
        )}
        {initiative.target_date != undefined && (
          <span>{t("plan-due", { date: day(initiative.target_date) })}</span>
        )}
      </div>
      {initiative.projects.length === 0 ? (
        <p className="vl-muted">{t("plan-initiative-no-projects")}</p>
      ) : (
        <ul className="vl-plan-list">
          {initiative.projects.map((p) => (
            <ProjectRow key={p.id} project={p} />
          ))}
        </ul>
      )}
    </li>
  );
}

function Initiatives({ api }: { readonly api: PlanningApi }) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const loaded = useLoaded(
    async () => {
      const [initiatives, projects] = await Promise.all([
        api.initiatives(),
        api.projects(),
      ]);
      return { initiatives, projects };
    },
    [api, version],
    failureText,
  );
  return (
    <Ready loaded={loaded}>
      {({ initiatives, projects }) => (
        <>
          <p>
            <Link to="/projects">{t("plan-projects")}</Link>
          </p>
          <NewInitiative
            api={api}
            projects={projects}
            onCreated={() => {
              setVersion((v) => v + 1);
            }}
          />
          {initiatives.length === 0 ? (
            <p className="vl-muted">{t("plan-initiatives-empty")}</p>
          ) : (
            <ul className="vl-plan-initiatives">
              {initiatives.map((initiative) => (
                <Initiative key={initiative.id} initiative={initiative} />
              ))}
            </ul>
          )}
        </>
      )}
    </Ready>
  );
}

/** Initiatives: groups of projects with rolled-up health and progress. */
export function InitiativesPage() {
  const t = useT();
  return (
    <ServerScreen
      api="planning"
      title={t("plan-initiatives")}
      className="vl-work vl-plan"
    >
      {(api) => <Initiatives api={api} />}
    </ServerScreen>
  );
}
