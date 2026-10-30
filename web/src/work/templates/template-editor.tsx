// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** One template: its summary row and its edit form (#144). */

import { useId, useState, type ComponentProps } from "react";

import { useT } from "../../i18n/i18n";
import type { ProjectOut } from "../planning/api";
import { priorityId } from "../planning/shared";
import type { TeamFile } from "../settings/team-file";

import {
  formOf,
  problemOf,
  repeatChoices,
  templateOf,
  type Template,
  type TemplateForm,
} from "./template-file";

function Facts({ template, file }: { readonly template: Template; readonly file: TeamFile }) {
  const t = useT();
  const status = file.workflow.statuses.find((each) => each.key === template.state);
  const facts = [
    template.title !== undefined && t("tpl-fact-title", { title: template.title }),
    status !== undefined && t("tpl-fact-status", { status: status.name }),
    template.priority !== undefined &&
      t("tpl-fact-priority", { priority: t(priorityId(template.priority)) }),
    template.estimate !== undefined &&
      t("tpl-fact-estimate", { estimate: String(template.estimate) }),
    template.assignee !== undefined && t("tpl-fact-assignee", { assignee: template.assignee }),
    (template.labels ?? []).length > 0 &&
      t("tpl-fact-labels", { labels: (template.labels ?? []).join(", ") }),
    template.repeat !== undefined && t("plan-template-repeat", { repeat: template.repeat }),
  ].filter((fact): fact is string => typeof fact === "string");
  if (facts.length === 0) return <p className="vl-muted">{t("tpl-no-defaults")}</p>;
  return (
    <ul className="vl-tpl-facts">
      {facts.map((fact) => (
        <li key={fact}>{fact}</li>
      ))}
    </ul>
  );
}

/** The form that makes or edits a template. */
export function TemplateEditor({
  file,
  before,
  otherNames,
  projects,
  busy,
  error,
  onSave,
  onCancel,
}: {
  readonly file: TeamFile;
  readonly before: Template | undefined;
  readonly otherNames: readonly string[];
  readonly projects: readonly ProjectOut[];
  readonly busy: boolean;
  readonly error: string | undefined;
  readonly onSave: (template: Template) => void;
  readonly onCancel: () => void;
}) {
  const t = useT();
  const id = useId();
  const [form, setForm] = useState<TemplateForm>(() => formOf(before));
  const [problem, setProblem] = useState<string>();
  const set = (field: keyof TemplateForm) => (value: string) => {
    setForm((current) => ({ ...current, [field]: value }));
  };
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const found = problemOf(form, otherNames);
    setProblem(found);
    if (found === undefined) onSave(templateOf(form, before));
  };
  const projectChoices =
    form.project === "" || projects.some((each) => each.id === form.project)
      ? projects
      : [...projects, { id: form.project, title: form.project } as ProjectOut];
  const field = (
    name: keyof TemplateForm,
    label: string,
    control: "input" | "textarea" = "input",
    hint?: string,
  ) => {
    const fieldId = `${id}-${name}`;
    const common = {
      id: fieldId,
      value: form[name],
      "aria-describedby": hint === undefined ? undefined : `${fieldId}-hint`,
      onChange: (event: { readonly target: { readonly value: string } }) => {
        set(name)(event.target.value);
      },
    };
    return (
      <span className="vl-settings-field">
        <label htmlFor={fieldId}>{label}</label>
        {control === "input" ? (
          <input {...common} autoComplete="off" />
        ) : (
          <textarea {...common} rows={6} />
        )}
        {hint !== undefined && (
          <small id={`${fieldId}-hint`} className="vl-muted">
            {hint}
          </small>
        )}
      </span>
    );
  };
  const select = (
    name: keyof TemplateForm,
    label: string,
    options: readonly (readonly [string, string])[],
  ) => (
    <span className="vl-settings-field">
      <label htmlFor={`${id}-${name}`}>{label}</label>
      <select
        id={`${id}-${name}`}
        value={form[name]}
        onChange={(event) => {
          set(name)(event.target.value);
        }}
      >
        {options.map(([value, text]) => (
          <option key={value} value={value}>
            {text}
          </option>
        ))}
      </select>
    </span>
  );
  const shown = problem === undefined ? error : t(problem);
  return (
    <form
      className="vl-tpl-editor"
      aria-label={before === undefined ? t("tpl-new") : t("tpl-edit-named", { name: before.name })}
      onSubmit={submit}
      noValidate
    >
      <div className="vl-settings-grid">
        {field("name", t("tpl-name"))}
        {field("title", t("tpl-title"), "input", t("tpl-title-hint"))}
        {select("state", t("tpl-status"), [
          ["", t("tpl-status-default")],
          ...file.workflow.statuses.map((status) => [status.key, status.name] as const),
        ])}
        {select("priority", t("plan-priority"), [
          ["", t("tpl-priority-unset")],
          ...[0, 1, 2, 3, 4].map((n) => [String(n), t(priorityId(n))] as const),
        ])}
        {field("estimate", t("plan-estimate"), "input", t("tpl-estimate-hint"))}
        {field("assignee", t("plan-assignee"))}
        {field("labels", t("plan-labels"))}
        {select("project", t("tpl-project"), [
          ["", t("tpl-project-none")],
          ...projectChoices.map((each) => [each.id, each.title] as const),
        ])}
        {select("repeat", t("plan-repeat"), [
          ["", t("plan-repeat-never")],
          ...repeatChoices(form.repeat).map(
            (r) => [r, t("plan-repeat-every", { repeat: r })] as const,
          ),
        ])}
      </div>
      {form.repeat !== "" && <p className="vl-muted">{t("plan-repeat-hint")}</p>}
      {field("body", t("tpl-body"), "textarea", t("tpl-body-hint"))}
      <div className="vl-settings-save">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {busy ? t("team-settings-saving") : t("tpl-save")}
        </button>
        <button type="button" className="vl-text-button" disabled={busy} onClick={onCancel}>
          {t("work-cancel")}
        </button>
        {shown !== undefined && (
          <p className="vl-field-error" role="alert">
            {shown}
          </p>
        )}
      </div>
    </form>
  );
}

/** A template's name, what it fills in, and its actions. */
export function TemplateRow({
  template,
  file,
  index,
  count,
  busy,
  onEdit,
  onCopy,
  onMove,
  onRemove,
}: {
  readonly template: Template;
  readonly file: TeamFile;
  readonly index: number;
  readonly count: number;
  readonly busy: boolean;
  readonly onEdit: () => void;
  readonly onCopy: () => void;
  readonly onMove: (step: -1 | 1) => void;
  readonly onRemove: () => void;
}) {
  const t = useT();
  const [confirming, setConfirming] = useState(false);
  const name = template.name;
  return (
    <li className="vl-tpl-row">
      <div className="vl-tpl-row-head">
        <h3>{name}</h3>
        <div className="vl-tpl-actions">
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            aria-label={t("tpl-edit-label", { name })}
            onClick={onEdit}
          >
            {t("tpl-edit")}
          </button>
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            aria-label={t("tpl-copy-label", { name })}
            onClick={onCopy}
          >
            {t("tpl-copy")}
          </button>
          <button
            type="button"
            className="vl-text-button"
            disabled={busy || index === 0}
            aria-label={t("tpl-move-up", { name })}
            onClick={() => {
              onMove(-1);
            }}
          >
            ↑
          </button>
          <button
            type="button"
            className="vl-text-button"
            disabled={busy || index === count - 1}
            aria-label={t("tpl-move-down", { name })}
            onClick={() => {
              onMove(1);
            }}
          >
            ↓
          </button>
          {confirming ? (
            <span className="vl-tpl-confirm" role="group" aria-label={t("tpl-remove-confirm")}>
              <button
                type="button"
                className="vl-text-button vl-danger"
                disabled={busy}
                onClick={onRemove}
              >
                {t("tpl-remove-yes", { name })}
              </button>
              <button
                type="button"
                className="vl-text-button"
                onClick={() => {
                  setConfirming(false);
                }}
              >
                {t("work-cancel")}
              </button>
            </span>
          ) : (
            <button
              type="button"
              className="vl-text-button"
              disabled={busy}
              aria-label={t("tpl-remove-label", { name })}
              onClick={() => {
                setConfirming(true);
              }}
            >
              {t("tpl-remove")}
            </button>
          )}
        </div>
      </div>
      <Facts template={template} file={file} />
      {template.body !== undefined && template.body !== "" && (
        <p className="vl-tpl-body">{template.body}</p>
      )}
    </li>
  );
}
