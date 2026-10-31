// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useParams } from "react-router";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { FormOut, IssueOut, PlanningApi, TeamOut } from "./api";
import { Failure, Field, text, useSubmit } from "./shared";

const formPath = (team: string, slug: string) =>
  `/asks/${encodeURIComponent(team)}/${encodeURIComponent(slug)}`;

function AskList({ api }: { readonly api: PlanningApi }) {
  const t = useT();
  const loaded = useLoaded(
    async () => {
      const teams = await api.teams();
      const forms = await Promise.all(
        teams.map(async (team) => ({ team, forms: await api.forms(team.key) })),
      );
      return forms.filter((entry) => entry.forms.length > 0);
    },
    [api],
    failureText,
  );
  return (
    <Ready loaded={loaded}>
      {(entries: { team: TeamOut; forms: FormOut[] }[]) =>
        entries.length === 0 ? (
          <p className="vl-muted">{t("plan-asks-empty")}</p>
        ) : (
          entries.map(({ team, forms }) => (
            <section key={team.key} aria-labelledby={`vl-asks-${team.key}`}>
              <h2 id={`vl-asks-${team.key}`}>{team.name}</h2>
              <ul className="vl-plan-list">
                {forms.map((form) => (
                  <li key={form.slug} className="vl-plan-row">
                    <Link to={formPath(team.key, form.slug)}>
                      <span className="vl-plan-row-title">{form.title}</span>
                      {form.description !== "" && (
                        <small className="vl-muted">{form.description}</small>
                      )}
                    </Link>
                  </li>
                ))}
              </ul>
            </section>
          ))
        )
      }
    </Ready>
  );
}

/** Asks: every team's intake forms (R-WK-16). */
export function AsksPage() {
  const t = useT();
  return (
    <ServerScreen
      api="planning"
      title={t("plan-asks")}
      className="vl-work vl-plan"
    >
      {(api) => (
        <>
          <p className="vl-muted">{t("plan-asks-intro")}</p>
          <AskList api={api} />
        </>
      )}
    </ServerScreen>
  );
}

function Answer({ field }: { readonly field: FormOut["fields"][number] }) {
  const t = useT();
  if (field.kind === "long_text") {
    return (
      <textarea
        name={field.key}
        rows={5}
        required={field.required}
        maxLength={16_384}
      />
    );
  }
  if (field.kind === "select") {
    return (
      <select name={field.key} required={field.required} defaultValue="">
        <option value="" disabled={field.required}>
          {t("plan-ask-choose")}
        </option>
        {field.options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    );
  }
  const email = field.kind === "email";
  return (
    <input
      name={field.key}
      type={email ? "email" : "text"}
      required={field.required}
      maxLength={16_384}
      autoComplete={email ? "email" : "off"}
    />
  );
}

function AskForm({
  api,
  team,
  form,
}: {
  readonly api: PlanningApi;
  readonly team: string;
  readonly form: FormOut;
}) {
  const t = useT();
  const [filed, setFiled] = useState<IssueOut>();
  const { busy, error, onSubmit } = useSubmit((data) => {
    const answers: Record<string, string> = {};
    for (const field of form.fields) {
      const answer = text(data, field.key);
      if (answer !== "") answers[field.key] = answer;
    }
    return api.submitForm(team, form.slug, answers);
  }, setFiled);
  if (filed !== undefined) {
    return (
      <div role="status">
        <p>{t("plan-ask-filed", { key: filed.key })}</p>
        <p>
          <Link to={`/issues/${encodeURIComponent(filed.key)}`}>
            {t("plan-ask-open")}
          </Link>{" "}
          ·{" "}
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setFiled(undefined);
            }}
          >
            {t("plan-ask-another")}
          </button>
        </p>
      </div>
    );
  }
  return (
    <form className="vl-plan-form vl-plan-ask" onSubmit={onSubmit}>
      {form.description !== "" && (
        <p className="vl-muted">{form.description}</p>
      )}
      {form.fields.map((field) => (
        <Field
          key={field.key}
          label={
            field.required
              ? t("plan-ask-required", { label: field.label })
              : field.label
          }
        >
          <Answer field={field} />
        </Field>
      ))}
      <button type="submit" className="vl-filled-button" disabled={busy}>
        {t("plan-ask-submit")}
      </button>
      <Failure error={error} />
    </form>
  );
}

function Ask({
  api,
  team,
  slug,
}: {
  readonly api: PlanningApi;
  readonly team: string;
  readonly slug: string;
}) {
  const t = useT();
  const loaded = useLoaded(() => api.forms(team), [api, team], failureText);
  return (
    <Ready loaded={loaded}>
      {(forms) => {
        const form = forms.find((f) => f.slug === slug);
        if (form === undefined)
          return <p className="vl-field-error">{t("plan-ask-missing")}</p>;
        return (
          <>
            <p className="vl-muted">
              <Link to="/asks">{t("plan-asks")}</Link> · {team}
            </p>
            <h1>{form.title}</h1>
            <AskForm api={api} team={team} form={form} />
          </>
        );
      }}
    </Ready>
  );
}

/** One intake form, filled in and filed as a triage issue. */
export function AskPage() {
  const { team = "", slug = "" } = useParams();
  return (
    <ServerScreen api="planning" className="vl-work vl-plan">
      {(api) => <Ask api={api} team={team} slug={slug} />}
    </ServerScreen>
  );
}
