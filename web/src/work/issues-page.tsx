// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Issues: a team's issues as a list or a board, grouped, ordered and
 * filtered (R-WK-4), with saved and shared views. Every setting lives in
 * the address, so a view can be bookmarked or sent as a link.
 */

import { useCallback, useRef, useState, type ComponentProps, type RefObject } from "react";
import { useNavigate } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { GroupOut, IssueOut, IssueQuery, TeamOut, ViewOut, WorkApi } from "./api";
import { IssueResults } from "./board";
import { PRIORITIES } from "./chips";
import { parse, serialize, type Group } from "./filter";
import { FilterBuilder } from "./filter-builder";
import { TeamPicker, useTeam, useWorkKeys, WorkNav } from "./nav";
import { SavedViews, Statuses } from "./saved-views";
import { CLOSED, GROUPS, ORDERS, Select, useSettings, type Settings } from "./view-settings";

import "./work.css";

export { stateName } from "./chips";

function NewIssueForm({
  api,
  team,
  input,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly input: RefObject<HTMLInputElement | null>;
}) {
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
        <input ref={input} name="title" autoComplete="off" required />
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

function Results({
  api,
  team,
  settings,
  version,
  onChanged,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly settings: Settings;
  readonly version: number;
  readonly onChanged: () => void;
}) {
  const query: IssueQuery = {
    team: team.key,
    group_by: settings.group,
    order_by: settings.order,
    show_closed: settings.closed,
    layout: settings.layout,
    ...(settings.filter === undefined ? {} : { filter: settings.filter }),
  };
  const loaded = useLoaded(
    () => api.queryIssues(query),
    [
      api,
      team.key,
      settings.group,
      settings.order,
      settings.closed,
      settings.layout,
      settings.filter,
      version,
    ],
    failureText,
  );
  const onMove = async (issue: IssueOut, group: string) => {
    await (settings.group === "status"
      ? api.setState(issue.key, group)
      : api.setPriority(
          issue.key,
          Math.max(0, PRIORITIES.indexOf(group as (typeof PRIORITIES)[number])),
        ));
    onChanged();
  };
  // While a change reloads the view, keep showing the last result so the
  // board does not flash and the moved card keeps its focus.
  const shape = JSON.stringify(query);
  const [last, setLast] = useState<{ shape: string; groups: readonly GroupOut[] }>();
  if (loaded.state === "ready" && last?.groups !== loaded.value) {
    setLast({ shape, groups: loaded.value });
  }
  const fresh = loaded.state === "ready" ? loaded.value : undefined;
  const kept = loaded.state === "loading" && last?.shape === shape ? last.groups : undefined;
  const shown = fresh ?? kept;
  if (shown === undefined) return <Ready loaded={loaded}>{() => <></>}</Ready>;
  return (
    <IssueResults
      groups={shown}
      team={team}
      layout={settings.layout}
      groupBy={settings.group}
      onMove={onMove}
    />
  );
}

function TeamIssues({ api, team }: { readonly api: WorkApi; readonly team: TeamOut }) {
  const t = useT();
  const [settings, change] = useSettings();
  const [showFilter, setShowFilter] = useState(false);
  const [version, setVersion] = useState(0);
  const newIssue = useRef<HTMLInputElement>(null);
  const focusNewIssue = useCallback(() => {
    newIssue.current?.focus();
  }, []);
  useWorkKeys(team.key, { newIssue: focusNewIssue });
  const parsed = parse(settings.filter);
  const conditions = parsed === undefined ? 0 : count(parsed);
  const apply = (view: ViewOut | undefined) => {
    if (view === undefined) {
      change({ view: undefined });
      return;
    }
    change({
      view: view.id,
      layout: view.layout,
      group: view.group_by,
      order: view.order_by,
      closed: view.show_closed,
      filter: view.filter === '{"and":[]}' ? undefined : view.filter,
    });
  };
  return (
    <>
      <NewIssueForm api={api} team={team} input={newIssue} />
      <div className="vl-work-toolbar" role="toolbar" aria-label={t("work-view-settings")}>
        <div className="vl-segmented" role="group" aria-label={t("work-layout")}>
          {(["list", "board"] as const).map((layout) => (
            <button
              key={layout}
              type="button"
              aria-pressed={settings.layout === layout}
              onClick={() => {
                change({ layout });
              }}
            >
              {t(`work-layout-${layout}`)}
            </button>
          ))}
        </div>
        <Select
          label={t("work-group-by")}
          value={settings.group}
          options={GROUPS}
          name={(group) => (group === "none" ? t("work-group-nothing") : t(`work-group-${group}`))}
          onChange={(group) => {
            change({ group });
          }}
        />
        <Select
          label={t("work-order-by")}
          value={settings.order as (typeof ORDERS)[number]}
          options={ORDERS}
          name={(order) => t(`work-order-${order}`)}
          onChange={(order) => {
            change({ order });
          }}
        />
        <Select
          label={t("work-closed")}
          value={settings.closed as (typeof CLOSED)[number]}
          options={CLOSED}
          name={(closed) => t(`work-closed-${closed}`)}
          onChange={(closed) => {
            change({ closed });
          }}
        />
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={showFilter}
          onClick={() => {
            setShowFilter((shown) => !shown);
          }}
        >
          {conditions === 0 ? t("work-filter") : t("work-filter-count", { count: conditions })}
        </button>
      </div>
      {showFilter &&
        (parsed === undefined ? (
          <p className="vl-muted">
            {t("work-filter-unsupported")}{" "}
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                change({ filter: undefined });
              }}
            >
              {t("work-filter-clear")}
            </button>
          </p>
        ) : (
          <FilterBuilder
            key={settings.filter ?? ""}
            value={parsed}
            statuses={team.statuses.map((status) => status.key)}
            onApply={(group: Group) => {
              change({ filter: serialize(group) });
            }}
          />
        ))}
      <SavedViews api={api} team={team} settings={settings} apply={apply} />
      <Results
        api={api}
        team={team}
        settings={settings}
        version={version}
        onChanged={() => {
          setVersion((v) => v + 1);
        }}
      />
      <Statuses team={team} />
    </>
  );
}

function count(group: Group): number {
  return group.children.reduce(
    (sum, child) => sum + (child.kind === "group" ? count(child) : 1),
    0,
  );
}

function Teams({ api, teams }: { readonly api: WorkApi; readonly teams: readonly TeamOut[] }) {
  const t = useT();
  const team = useTeam(teams);
  if (team === undefined) return <p className="vl-muted">{t("issues-no-teams")}</p>;
  return (
    <>
      <div className="vl-work-filters">
        <TeamPicker teams={teams} team={team} reset />
      </div>
      <TeamIssues key={team.key} api={api} team={team} />
    </>
  );
}

function IssuesForApi({ api }: { readonly api: WorkApi }) {
  const loaded = useLoaded(() => api.teams(), [api], failureText);
  return <Ready loaded={loaded}>{(teams) => <Teams api={api} teams={teams} />}</Ready>;
}

/** Issues: a team's issues as a list or board, and a form to file a new one. */
export function IssuesPage() {
  const t = useT();
  return (
    <ServerScreen api="work" title={t("issues")} className="vl-work vl-work-wide">
      {(api) => (
        <>
          <WorkNav />
          <IssuesForApi api={api} />
        </>
      )}
    </ServerScreen>
  );
}
