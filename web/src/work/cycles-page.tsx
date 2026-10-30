// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Cycles (R-WK-8, D16): the team's current cycle with its burn-up, the
 * velocity of the last three finished cycles, and every cycle with a link
 * to its issues on the board.
 */

import { Link } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { CycleOut, CyclesOut, TeamOut, WorkApi } from "./api";
import { BurnUpChart } from "./burn-up";
import { TeamPicker, useTeam, useWorkKeys, WorkNav } from "./nav";

import "./work.css";

/** Today's date as `YYYY-MM-DD` in the browser's zone. */
function today(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${String(now.getFullYear())}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** Where a cycle stands on `on`: cycles end the day before `ends_on`. */
export function cycleState(
  cycle: CycleOut,
  on: string,
): "past" | "current" | "upcoming" {
  if (cycle.ends_on <= on) return "past";
  return cycle.starts_on <= on ? "current" : "upcoming";
}

function boardLink(team: string, cycle: string): string {
  const filter = JSON.stringify({ field: "cycle", op: "is", values: [cycle] });
  const query = new URLSearchParams({
    team,
    layout: "board",
    group: "status",
    filter,
    closed: "all",
  });
  return `/issues?${query.toString()}`;
}

function name(cycle: CycleOut): string {
  return cycle.name ?? cycle.key;
}

function Cycles({
  team,
  found,
  on,
}: {
  readonly team: TeamOut;
  readonly found: CyclesOut;
  readonly on: string;
}) {
  const t = useT();
  if (!team.cycles && found.cycles.length === 0) {
    return (
      <p className="vl-muted">{t("work-cycles-off", { team: team.name })}</p>
    );
  }
  const current = found.cycles.find(
    (cycle) => cycleState(cycle, on) === "current",
  );
  const velocity = found.velocity ?? undefined;
  return (
    <>
      <p className="vl-work-stat">
        {velocity === undefined
          ? t("work-velocity-none")
          : t("work-velocity", { points: Math.round(velocity * 10) / 10 })}
      </p>
      {current === undefined ? (
        <p className="vl-muted">{t("work-cycle-none-current")}</p>
      ) : (
        <section aria-labelledby="current-cycle" className="vl-cycle-current">
          <h2 id="current-cycle">
            {t("work-cycle-current", { name: name(current) })}
          </h2>
          <p className="vl-muted">
            {t("work-cycle-dates", {
              start: current.starts_on,
              end: current.ends_on,
            })}{" "}
            ·{" "}
            <Link to={boardLink(team.key, current.key)}>
              {t("work-cycle-open-board")}
            </Link>
          </p>
          <BurnUpChart points={current.burn_up} title={name(current)} />
        </section>
      )}
      <h2>{t("work-cycles-all")}</h2>
      <table className="vl-cycle-table">
        <thead>
          <tr>
            <th scope="col">{t("work-cycle")}</th>
            <th scope="col">{t("work-cycle-when")}</th>
            <th scope="col">{t("work-cycle-state")}</th>
            <th scope="col">{t("work-cycle-points")}</th>
          </tr>
        </thead>
        <tbody>
          {found.cycles.toReversed().map((cycle) => {
            const state = cycleState(cycle, on);
            const last = cycle.burn_up.at(-1);
            return (
              <tr
                key={cycle.key}
                className={
                  state === "current" ? "vl-cycle-row-current" : undefined
                }
              >
                <th scope="row">
                  <Link to={boardLink(team.key, cycle.key)}>{name(cycle)}</Link>
                </th>
                <td>
                  {t("work-cycle-dates", {
                    start: cycle.starts_on,
                    end: cycle.ends_on,
                  })}
                </td>
                <td>{t(`work-cycle-state-${state}`)}</td>
                <td>
                  {t("work-cycle-done-of", {
                    completed: cycle.completed_points,
                    scope: last?.scope ?? cycle.completed_points,
                  })}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </>
  );
}

function TeamCycles({
  api,
  team,
  on,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly on: string;
}) {
  useWorkKeys(team.key);
  const loaded = useLoaded(
    () => api.cycles(team.key),
    [api, team.key],
    failureText,
  );
  return (
    <Ready loaded={loaded}>
      {(found) => <Cycles team={team} found={found} on={on} />}
    </Ready>
  );
}

function CyclesForTeams({
  api,
  teams,
  on,
}: {
  readonly api: WorkApi;
  readonly teams: readonly TeamOut[];
  readonly on: string;
}) {
  const t = useT();
  const team = useTeam(teams);
  if (team === undefined)
    return <p className="vl-muted">{t("issues-no-teams")}</p>;
  return (
    <>
      <div className="vl-work-filters">
        <TeamPicker teams={teams} team={team} />
      </div>
      <TeamCycles key={team.key} api={api} team={team} on={on} />
    </>
  );
}

/** The cycles screen; `on` fixes today for tests. */
export function CyclesPage({ on = today() }: { readonly on?: string }) {
  const t = useT();
  return (
    <ServerScreen api="work" title={t("work-cycles")} className="vl-work">
      {(api) => (
        <>
          <WorkNav />
          <CyclesLoader api={api} on={on} />
        </>
      )}
    </ServerScreen>
  );
}

function CyclesLoader({
  api,
  on,
}: {
  readonly api: WorkApi;
  readonly on: string;
}) {
  const loaded = useLoaded(() => api.teams(), [api], failureText);
  return (
    <Ready loaded={loaded}>
      {(teams) => <CyclesForTeams api={api} teams={teams} on={on} />}
    </Ready>
  );
}
