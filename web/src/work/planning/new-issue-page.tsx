// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { useNavigate, useSearchParams } from "react-router";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { IssueDraft, PlanningApi, TeamOut, TemplateOut } from "./api";
import { Failure, Field, priorityId, text, useSubmit } from "./shared";

/** The repeats the form offers; a URL may ask for any other (`3d`). */
const REPEATS = ["", "1d", "1w", "2w", "1m", "3m"] as const;

const PRIORITIES = ["0", "1", "2", "3", "4"] as const;

/**
 * A draft from a create URL (R-WK-13), such as
 * `/issues/new?team=ENG&title=Fix+login&labels=bug,web&priority=2`.
 * Unknown parameters are ignored; nothing is created until a person submits.
 */
export function draftFromUrl(
  parameters: URLSearchParams,
): Partial<Record<keyof IssueDraft, string>> {
  const keys = [
    "team",
    "title",
    "description",
    "template",
    "priority",
    "estimate",
    "assignee",
    "labels",
    "project",
    "milestone",
    "due",
    "repeat",
  ] as const;
  const out: Partial<Record<keyof IssueDraft, string>> = {};
  for (const key of keys) {
    const value = parameters.get(key);
    if (value !== null && value.length <= 16_384) out[key] = value;
  }
  return out;
}

/** The draft a submitted create form describes. */
function draftFrom(form: FormData, team: string, template: string): IssueDraft {
  const priority = text(form, "priority");
  return {
    team,
    title: text(form, "title"),
    description: text(form, "description"),
    template,
    priority: priority === "" ? undefined : Number(priority),
    estimate: text(form, "estimate"),
    assignee: text(form, "assignee").replace(/^@/, ""),
    labels: text(form, "labels")
      .split(",")
      .map((label) => label.trim())
      .filter((label) => label !== ""),
    project: text(form, "project"),
    milestone: text(form, "milestone"),
    due: text(form, "due"),
    repeat: text(form, "repeat"),
  };
}

/** The repeat choices, plus one a URL asked for. */
function repeatChoices(asked: string | undefined): readonly string[] {
  const known: readonly string[] = REPEATS;
  return asked === undefined || known.includes(asked) ? known : [...known, asked];
}

function TemplatePreview({ template }: { readonly template: TemplateOut | undefined }) {
  const t = useT();
  if (template === undefined) return;
  const facts = [
    template.title != undefined && t("plan-template-title", { title: template.title }),
    template.priority != undefined &&
      t("plan-suggest-priority", {
        priority: t(priorityId(template.priority)),
      }),
    template.labels.length > 0 && t("plan-suggest-labels", { labels: template.labels.join(", ") }),
    template.assignee != undefined &&
      t("plan-suggest-assignee", { assignee: `@${template.assignee}` }),
    template.repeat != undefined && t("plan-template-repeat", { repeat: template.repeat }),
  ].filter((f): f is string => typeof f === "string");
  return (
    <div className="vl-plan-template" aria-live="polite">
      {facts.length > 0 && (
        <ul>
          {facts.map((f) => (
            <li key={f}>{f}</li>
          ))}
        </ul>
      )}
      {template.body !== "" && <p className="vl-issue-description">{template.body}</p>}
    </div>
  );
}

/** The fields a URL may prefill. */
function DraftFields({ asked }: { readonly asked: ReturnType<typeof draftFromUrl> }) {
  const t = useT();
  const repeats = repeatChoices(asked.repeat);
  return (
    <>
      <Field label={t("plan-title")}>
        <input
          name="title"
          required
          maxLength={500}
          defaultValue={asked.title}
          autoComplete="off"
        />
      </Field>
      <Field label={t("plan-description")}>
        <textarea name="description" rows={6} defaultValue={asked.description} />
      </Field>
      <Field label={t("plan-priority")}>
        <select name="priority" defaultValue={asked.priority ?? ""}>
          <option value="">{t("plan-priority-default")}</option>
          {PRIORITIES.map((p, index) => (
            <option key={p} value={p}>
              {t(priorityId(index))}
            </option>
          ))}
        </select>
      </Field>
      <Field label={t("plan-estimate")}>
        <input name="estimate" defaultValue={asked.estimate} autoComplete="off" />
      </Field>
      <Field label={t("plan-assignee")}>
        <input
          name="assignee"
          placeholder="@handle"
          defaultValue={asked.assignee}
          autoComplete="off"
        />
      </Field>
      <Field label={t("plan-labels")}>
        <input
          name="labels"
          placeholder="bug, web"
          defaultValue={asked.labels}
          autoComplete="off"
        />
      </Field>
      <input type="hidden" name="project" value={asked.project ?? ""} />
      <input type="hidden" name="milestone" value={asked.milestone ?? ""} />
      <Field label={t("plan-due-date")}>
        <input name="due" type="date" defaultValue={asked.due} />
      </Field>
      <Field label={t("plan-repeat")}>
        <select name="repeat" defaultValue={asked.repeat ?? ""}>
          {repeats.map((r) => (
            <option key={r} value={r}>
              {r === "" ? t("plan-repeat-never") : t("plan-repeat-every", { repeat: r })}
            </option>
          ))}
        </select>
      </Field>
    </>
  );
}

function NewIssue({
  api,
  teams,
}: {
  readonly api: PlanningApi;
  readonly teams: readonly TeamOut[];
}) {
  const t = useT();
  const navigate = useNavigate();
  const [parameters] = useSearchParams();
  const asked = draftFromUrl(parameters);
  const [team, setTeam] = useState(
    teams.find((x) => x.key === asked.team)?.key ?? teams[0]?.key ?? "",
  );
  const [templateName, setTemplateName] = useState(asked.template ?? "");
  const templates = useLoaded(
    () => (team === "" ? Promise.resolve([]) : api.templates(team)),
    [api, team],
    failureText,
  );
  const { busy, error, onSubmit } = useSubmit(
    (form) => {
      const title = text(form, "title");
      if (title === "" || team === "") return;
      return api.createIssue(draftFrom(form, team, templateName));
    },
    (issue) => {
      void navigate(`/issues/${encodeURIComponent(issue.key)}`);
    },
  );
  if (teams.length === 0) return <p className="vl-muted">{t("issues-no-teams")}</p>;
  return (
    <form className="vl-plan-form vl-plan-issue-form" onSubmit={onSubmit}>
      <Field label={t("issues-team")}>
        <select
          name="team"
          value={team}
          onChange={(event) => {
            setTeam(event.target.value);
            setTemplateName("");
          }}
        >
          {teams.map((x) => (
            <option key={x.key} value={x.key}>
              {x.name}
            </option>
          ))}
        </select>
      </Field>
      <Field label={t("plan-template")}>
        <select
          value={templateName}
          onChange={(event) => {
            setTemplateName(event.target.value);
          }}
          disabled={templates.state !== "ready" || templates.value.length === 0}
        >
          <option value="">{t("plan-template-none")}</option>
          {templates.state === "ready" &&
            templates.value.map((x) => (
              <option key={x.name} value={x.name}>
                {x.name}
              </option>
            ))}
        </select>
      </Field>
      <TemplatePreview
        template={
          templates.state === "ready"
            ? templates.value.find((x) => x.name === templateName)
            : undefined
        }
      />
      <DraftFields asked={asked} />
      <p className="vl-muted">{t("plan-repeat-hint")}</p>
      <button type="submit" className="vl-filled-button" disabled={busy}>
        {t("issues-create")}
      </button>
      <Failure error={error} />
    </form>
  );
}

function NewIssueForApi({ api }: { readonly api: PlanningApi }) {
  const loaded = useLoaded(() => api.teams(), [api], failureText);
  return <Ready loaded={loaded}>{(teams) => <NewIssue api={api} teams={teams} />}</Ready>;
}

/** New issue: every field, a template, a repeat, prefilled from the URL. */
export function NewIssuePage() {
  const t = useT();
  return (
    <ServerScreen api="planning" title={t("plan-issue-new")} className="vl-work vl-plan">
      {(api) => <NewIssueForApi api={api} />}
    </ServerScreen>
  );
}
