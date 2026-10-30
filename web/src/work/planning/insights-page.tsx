// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { InsightsOut, InsightsQuestion, PlanningApi, TeamOut } from "./api";
import { Field, isoDay } from "./shared";

const MEASURES = ["lead_time", "cycle_time", "triage_time", "throughput", "created"] as const;
const PIVOTS = [
  "none",
  "team",
  "assignee",
  "priority",
  "label",
  "project",
  "cycle",
  "estimate",
] as const;

/** Whether a measure counts issues rather than timing them. */
const counts = (measure: string) => measure === "throughput" || measure === "created";

/** Hours as people read them: "5 h", "2.5 d". */
export function duration(hours?: number | null): string {
  if (hours == undefined) return "–";
  if (hours < 48) return `${String(Math.round(hours * 10) / 10)} h`;
  return `${String(Math.round((hours / 24) * 10) / 10)} d`;
}

interface Row {
  readonly key: string;
  readonly count: number;
  readonly p50: number | null | undefined;
  readonly p85: number | null | undefined;
}

function Chart({
  rows,
  counting,
  title,
}: {
  readonly rows: readonly Row[];
  readonly counting: boolean;
  readonly title: string;
}) {
  const t = useT();
  const value = (r: Row) => (counting ? r.count : (r.p85 ?? 0));
  const max = Math.max(1, ...rows.map((r) => value(r)));
  // The table scrolls sideways when zoomed or on a narrow screen. A region
  // that scrolls must take keyboard focus, or keyboard users cannot reach
  // the columns it hides (WCAG 2.1.1, axe `scrollable-region-focusable`).
  return (
    // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- see above
    <figure className="vl-plan-chart" tabIndex={0}>
      <figcaption>{title}</figcaption>
      <table>
        <thead>
          <tr>
            <th scope="col">{t("plan-insights-group")}</th>
            <th scope="col">{t("plan-insights-count")}</th>
            {!counting && <th scope="col">{t("plan-insights-p50")}</th>}
            {!counting && <th scope="col">{t("plan-insights-p85")}</th>}
            <th scope="col" aria-hidden="true" className="vl-plan-chart-bar-col" />
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.key}>
              <th scope="row">{r.key}</th>
              <td>{r.count}</td>
              {!counting && <td>{duration(r.p50)}</td>}
              {!counting && <td>{duration(r.p85)}</td>}
              <td aria-hidden="true" className="vl-plan-chart-bar-col">
                <span
                  className="vl-plan-chart-bar"
                  style={{ inlineSize: `${String((value(r) / max) * 100)}%` }}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  );
}

function Answer({
  answer,
  question,
}: {
  readonly answer: InsightsOut;
  readonly question: InsightsQuestion;
}) {
  const t = useT();
  const counting = counts(question.measure);
  const none = t("plan-insights-none");
  const rows: Row[] =
    question.pivot === "none"
      ? [
          {
            key: t("plan-insights-all"),
            count: answer.overall.count,
            p50: answer.overall.p50_hours,
            p85: answer.overall.p85_hours,
          },
        ]
      : answer.buckets.map((b) => ({
          key: b.key ?? none,
          count: b.overall.count,
          p50: b.overall.p50_hours,
          p85: b.overall.p85_hours,
        }));
  const series = answer.buckets[0]?.series ?? [];
  const seriesRows: Row[] =
    question.interval === undefined || question.pivot !== "none"
      ? []
      : series.map((p) => ({
          key: p.start,
          count: p.stats.count,
          p50: p.stats.p50_hours,
          p85: p.stats.p85_hours,
        }));
  if (answer.overall.count === 0) return <p className="vl-muted">{t("plan-insights-empty")}</p>;
  return (
    <>
      <p className="vl-plan-summary">
        <span>{t("plan-insights-total", { count: answer.overall.count })}</span>
        {!counting && (
          <span>{t("plan-insights-median", { value: duration(answer.overall.p50_hours) })}</span>
        )}
        {!counting && (
          <span>{t("plan-insights-85", { value: duration(answer.overall.p85_hours) })}</span>
        )}
      </p>
      <Chart rows={rows} counting={counting} title={t(`plan-measure-${question.measure}`)} />
      {seriesRows.length > 0 && (
        <Chart rows={seriesRows} counting={counting} title={t("plan-insights-over-time")} />
      )}
    </>
  );
}

function Insights({
  api,
  teams,
}: {
  readonly api: PlanningApi;
  readonly teams: readonly TeamOut[];
}) {
  const t = useT();
  const [question, setQuestion] = useState<InsightsQuestion>({
    measure: "lead_time",
    pivot: "none",
    from: isoDay(-90),
    to: isoDay(1),
    interval: "week",
  });
  const set = (patch: Partial<InsightsQuestion>) => {
    setQuestion((q) => ({ ...q, ...patch }));
  };
  const loaded = useLoaded(
    () => api.insights(question),
    [api, JSON.stringify(question)],
    failureText,
  );
  return (
    <>
      <div className="vl-plan-form">
        <Field label={t("plan-insights-measure")}>
          <select
            value={question.measure}
            onChange={(event) => {
              set({ measure: event.target.value });
            }}
          >
            {MEASURES.map((m) => (
              <option key={m} value={m}>
                {t(`plan-measure-${m}`)}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("plan-insights-pivot")}>
          <select
            value={question.pivot}
            onChange={(event) => {
              set({ pivot: event.target.value });
            }}
          >
            {PIVOTS.map((p) => (
              <option key={p} value={p}>
                {t(`plan-pivot-${p}`)}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("issues-team")}>
          <select
            value={question.team ?? ""}
            onChange={(event) => {
              set({ team: event.target.value === "" ? undefined : event.target.value });
            }}
          >
            <option value="">{t("plan-insights-all-teams")}</option>
            {teams.map((team) => (
              <option key={team.key} value={team.key}>
                {team.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("plan-insights-from")}>
          <input
            type="date"
            value={question.from}
            onChange={(event) => {
              if (event.target.value !== "") set({ from: event.target.value });
            }}
          />
        </Field>
        <Field label={t("plan-insights-to")}>
          <input
            type="date"
            value={question.to}
            onChange={(event) => {
              if (event.target.value !== "") set({ to: event.target.value });
            }}
          />
        </Field>
        <Field label={t("plan-insights-interval")}>
          <select
            value={question.interval ?? ""}
            onChange={(event) => {
              set({ interval: event.target.value === "" ? undefined : event.target.value });
            }}
          >
            <option value="">{t("plan-interval-none")}</option>
            <option value="week">{t("plan-interval-week")}</option>
            <option value="month">{t("plan-interval-month")}</option>
          </select>
        </Field>
      </div>
      <div aria-live="polite">
        <Ready loaded={loaded}>{(answer) => <Answer answer={answer} question={question} />}</Ready>
      </div>
    </>
  );
}

function InsightsForApi({ api }: { readonly api: PlanningApi }) {
  const loaded = useLoaded(() => api.teams(), [api], failureText);
  return <Ready loaded={loaded}>{(teams) => <Insights api={api} teams={teams} />}</Ready>;
}

/** Insights: lead, cycle and triage time and throughput, by any pivot (R-WK-15). */
export function InsightsPage() {
  const t = useT();
  return (
    <ServerScreen api="planning" title={t("plan-insights")} className="vl-work vl-plan">
      {(api) => <InsightsForApi api={api} />}
    </ServerScreen>
  );
}
