// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Team settings (#524): create a team, and edit a team's statuses, cycles,
 * triage rules, SLAs, estimates and Git mapping. The screen edits the
 * team's settings file as JSON and saves it whole through
 * `PUT /v1/work/teams/{team}`, which validates it before anything changes.
 * Teams are archived and restored here too (#538, D303).
 */

import { useState, type ComponentProps } from "react";
import { useSearchParams } from "react-router";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import type { IssueOut, TeamOut, WorkApi } from "../api";
import { fieldText } from "../form";
import { TeamPicker, useTeam, useWorkKeys, WorkNav } from "../nav";

import { ArchiveSection } from "./archive";
import { SlaSection, TriageSection } from "./rules";
import { CyclesSection, EstimatesSection, GeneralSection, GitSection } from "./sections";
import { StatusesSection, type Moves } from "./statuses";
import { pruneGit, type TeamFile } from "./team-file";

import "./settings.css";

/** The issues in a status, closed ones included. */
async function issuesIn(api: WorkApi, team: string, status: string): Promise<IssueOut[]> {
  const groups = await api.queryIssues({
    team,
    group_by: "none",
    show_closed: "all",
    filter: JSON.stringify({ field: "status", op: "is", values: [status] }),
  });
  return groups.flatMap((group) => group.issues);
}

/** Moves every issue out of the statuses being removed. */
async function runMoves(api: WorkApi, team: string, moves: Moves): Promise<void> {
  for (const [from, to] of Object.entries(moves)) {
    // A query returns one page of issues; go again until none are left.
    for (let round = 0; round < 50; round += 1) {
      const left = await issuesIn(api, team, from);
      if (left.length === 0) break;
      for (const issue of left) await api.setState(issue.key, to);
    }
  }
}

function SaveBar({
  dirty,
  state,
  error,
  onSave,
  onDiscard,
}: {
  readonly dirty: boolean;
  readonly state: "idle" | "saving" | "saved";
  readonly error: string | undefined;
  readonly onSave: () => void;
  readonly onDiscard: () => void;
}) {
  const t = useT();
  return (
    <div className="vl-settings-save">
      <button
        type="button"
        className="vl-filled-button"
        disabled={!dirty || state === "saving"}
        onClick={onSave}
      >
        {state === "saving" ? t("team-settings-saving") : t("team-settings-save")}
      </button>
      <button
        type="button"
        className="vl-text-button"
        disabled={!dirty || state === "saving"}
        onClick={onDiscard}
      >
        {t("team-settings-discard")}
      </button>
      <p role="status" className="vl-muted">
        {state === "saved" ? t("team-settings-saved") : ""}
      </p>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function Editor({
  api,
  team,
  initial,
  justSaved,
  onSaved,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly initial: TeamFile;
  /** Whether this copy was loaded right after a save, to say so. */
  readonly justSaved: boolean;
  readonly onSaved: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const [moves, setMoves] = useState<Moves>({});
  const [state, setState] = useState<"idle" | "saving" | "saved">(justSaved ? "saved" : "idle");
  const [error, setError] = useState<string>();
  const edit = (change: (file: TeamFile) => TeamFile) => {
    setState("idle");
    setDraft((current) => change(current));
  };
  const save = () => {
    setState("saving");
    setError(undefined);
    runMoves(api, team.key, moves)
      .then(() => api.saveTeamSettings(team.key, pruneGit(draft)))
      .then(
        () => {
          onSaved();
        },
        (error_: unknown) => {
          setState("idle");
          setError(failureText(error_));
        },
      );
  };
  return (
    <>
      <GeneralSection file={draft} edit={edit} />
      <StatusesSection
        file={draft}
        edit={edit}
        countIssues={async (status) => {
          const found = await issuesIn(api, team.key, status);
          return found.length;
        }}
        onMove={(from, to) => {
          setMoves((current) => ({ ...current, [from]: to }));
        }}
      />
      <CyclesSection file={draft} edit={edit} />
      <TriageSection file={draft} edit={edit} />
      <SlaSection file={draft} edit={edit} />
      <EstimatesSection file={draft} edit={edit} />
      <GitSection file={draft} edit={edit} />
      <SaveBar
        dirty={draft !== initial}
        state={state}
        error={error}
        onSave={save}
        onDiscard={() => {
          setDraft(initial);
          setMoves({});
        }}
      />
    </>
  );
}

function TeamEditor({
  api,
  team,
  onArchived,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly onArchived: (key: string) => void;
}) {
  const [version, setVersion] = useState(0);
  const [saved, setSaved] = useState(false);
  useWorkKeys(team.key);
  const loaded = useLoaded(
    () => api.teamSettings(team.key),
    [api, team.key, team.archived, version],
    failureText,
  );
  return (
    <>
      <Ready loaded={loaded}>
        {(file) => (
          <Editor
            key={`${team.key}:${String(team.archived)}:${String(version)}`}
            api={api}
            team={team}
            initial={file}
            justSaved={saved}
            onSaved={() => {
              setSaved(true);
              setVersion((v) => v + 1);
            }}
          />
        )}
      </Ready>
      <ArchiveSection
        api={api}
        team={team}
        onChanged={() => {
          onArchived(team.key);
        }}
      />
    </>
  );
}

function NewTeam({
  api,
  onCreated,
}: {
  readonly api: WorkApi;
  readonly onCreated: (key: string) => void;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string>();
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const members = fieldText(data, "members")
      .split(",")
      .map((each) => each.trim().replace(/^@/, ""))
      .filter((each) => each !== "");
    setError(undefined);
    api
      .createTeam({
        key: fieldText(data, "key").toUpperCase(),
        name: fieldText(data, "name"),
        members,
      })
      .then(
        (team) => {
          setOpen(false);
          onCreated(team.key);
        },
        (error_: unknown) => {
          setError(failureText(error_));
        },
      );
  };
  if (!open) {
    return (
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setOpen(true);
        }}
      >
        {t("team-settings-new")}
      </button>
    );
  }
  return (
    <form className="vl-work-add" onSubmit={submit} aria-label={t("team-settings-new")}>
      <label>
        {t("team-settings-name")}
        <input name="name" required autoComplete="off" />
      </label>
      <label>
        {t("team-settings-key")}
        <input
          name="key"
          required
          autoComplete="off"
          pattern="[A-Za-z][A-Za-z0-9]{1,6}"
          title={t("team-settings-key-hint")}
        />
      </label>
      <label>
        {t("team-settings-members")}
        <input name="members" autoComplete="off" />
      </label>
      <button type="submit" className="vl-filled-button">
        {t("team-settings-create")}
      </button>
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setOpen(false);
        }}
      >
        {t("work-cancel")}
      </button>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

function TeamList({
  api,
  teams,
  showArchived,
  onShowArchived,
  onCreated,
  onArchived,
}: {
  readonly api: WorkApi;
  readonly teams: readonly TeamOut[];
  readonly showArchived: boolean;
  readonly onShowArchived: (show: boolean) => void;
  readonly onCreated: (key: string) => void;
  readonly onArchived: (key: string) => void;
}) {
  const t = useT();
  const team = useTeam(teams);
  const named = teams.map((each) =>
    each.archived ? { ...each, name: t("team-settings-archived-name", { name: each.name }) } : each,
  );
  const picked = named.find((each) => each.key === team?.key);
  return (
    <>
      <div className="vl-work-filters">
        {picked !== undefined && <TeamPicker teams={named} team={picked} reset />}
        <label className="vl-work-field">
          <input
            type="checkbox"
            checked={showArchived}
            onChange={(event) => {
              onShowArchived(event.target.checked);
            }}
          />
          {t("team-settings-show-archived")}
        </label>
        <NewTeam api={api} onCreated={onCreated} />
      </div>
      {team === undefined ? (
        <p className="vl-muted">{t("issues-no-teams")}</p>
      ) : (
        <TeamEditor key={team.key} api={api} team={team} onArchived={onArchived} />
      )}
    </>
  );
}

function Teams({ api }: { readonly api: WorkApi }) {
  const [version, setVersion] = useState(0);
  const [showArchived, setShowArchived] = useState(false);
  const [parameters, setParameters] = useSearchParams();
  // A team opened by its address stays reachable once archived.
  const loaded = useLoaded(() => api.teams({ includeArchived: true }), [api, version], failureText);
  const wanted = parameters.get("team");
  return (
    <Ready loaded={loaded}>
      {(all) => (
        <TeamList
          api={api}
          teams={all.filter((team) => showArchived || !team.archived || team.key === wanted)}
          showArchived={showArchived}
          onShowArchived={setShowArchived}
          onCreated={(key) => {
            setParameters({ team: key }, { replace: true });
            setVersion((v) => v + 1);
          }}
          onArchived={(key) => {
            // Stay on the team: archived, it shows with Restore.
            setParameters({ team: key }, { replace: true });
            setVersion((v) => v + 1);
          }}
        />
      )}
    </Ready>
  );
}

/** The team settings screen. */
export function TeamSettingsPage() {
  const t = useT();
  return (
    <ServerScreen api="work" title={t("team-settings")} className="vl-work">
      {(api) => (
        <>
          <WorkNav />
          <Teams api={api} />
        </>
      )}
    </ServerScreen>
  );
}
