// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** The Work screens' tabs and team picker, and the `G` chords between them. */

import { useId, useMemo } from "react";
import { NavLink, useNavigate, useSearchParams } from "react-router";

import { useT } from "../i18n/i18n";
import { useScreenCommands } from "../keyboard/keyboard-root";
import type { Scope } from "../keyboard/keymap";
import {
  SCREEN_DESTINATIONS,
  workCommands,
  type WorkActions,
} from "../keyboard/work-commands";

import type { TeamOut } from "./api";

const TABS = [
  ["/issues", "work-tab-issues"],
  ["/issues/mine", "work-tab-mine"],
  ["/issues/triage", "work-tab-triage"],
  ["/issues/cycles", "work-tab-cycles"],
  ["/issues/templates", "work-tab-templates"],
  ["/issues/settings", "work-tab-settings"],
] as const;

const ISSUE_SCOPES: readonly Scope[] = ["issues"];

/** The tabs across the Work screens; the team carries over. */
export function WorkNav() {
  const t = useT();
  const [parameters] = useSearchParams();
  const team = parameters.get("team");
  const query = team === null ? "" : `?team=${encodeURIComponent(team)}`;
  return (
    <nav className="vl-work-tabs" aria-label={t("work-tabs")}>
      {TABS.map(([path, label]) => (
        <NavLink key={path} to={`${path}${query}`} end>
          {t(label)}
        </NavLink>
      ))}
    </nav>
  );
}

/** The team named in the address, or the first team. */
export function useTeam(teams: readonly TeamOut[]): TeamOut | undefined {
  const [parameters] = useSearchParams();
  const asked = parameters.get("team");
  return teams.find((each) => each.key === asked) ?? teams[0];
}

/** Picks the team; other address settings are kept unless `reset` is set. */
export function TeamPicker({
  teams,
  team,
  reset = false,
}: {
  readonly teams: readonly TeamOut[];
  readonly team: TeamOut;
  readonly reset?: boolean;
}) {
  const t = useT();
  const id = useId();
  const [parameters, setParameters] = useSearchParams();
  return (
    <span className="vl-work-field">
      <label htmlFor={id}>{t("issues-team")}</label>
      <select
        id={id}
        value={team.key}
        onChange={(event) => {
          const next = reset
            ? new URLSearchParams()
            : new URLSearchParams(parameters);
          next.set("team", event.target.value);
          next.delete("view");
          setParameters(next, { replace: true });
        }}
      >
        {teams.map((each) => (
          <option key={each.key} value={each.key}>
            {each.name}
          </option>
        ))}
      </select>
    </span>
  );
}

/**
 * Registers the Work keyboard model for a screen: the `G` chords, plus
 * whatever else the screen can do.
 */
export function useWorkKeys(
  team: string | undefined,
  actions: Omit<WorkActions, "goTo"> = {},
) {
  const navigate = useNavigate();
  const { newIssue } = actions;
  const commands = useMemo(() => {
    const query = team === undefined ? "" : `?team=${encodeURIComponent(team)}`;
    return workCommands({
      ...(newIssue === undefined ? {} : { newIssue }),
      destinations: SCREEN_DESTINATIONS,
      goTo(destination) {
        const path = {
          inbox: "/inbox",
          "my-work": "/my-work",
          board: `/issues${query}${query === "" ? "?" : "&"}layout=board`,
          cycle: `/issues/cycles${query}`,
          triage: `/issues/triage${query}`,
        }[destination];
        void navigate(path);
      },
    });
  }, [navigate, team, newIssue]);
  useScreenCommands(commands, ISSUE_SCOPES);
}
