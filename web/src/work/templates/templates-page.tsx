// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Templates and recurring issues (#144, R-WK-12, D307-D308). A team's
 * templates are made, edited, copied, reordered and removed here; each
 * change saves the team's settings file whole through
 * `PUT /v1/work/teams/{team}`, which checks it first. Below them, the
 * team's repeating issues, with their repeat changed or stopped in place.
 */

import { useState } from "react";

import { failureText } from "../../data/failure";
import { Ready, ServerScreen, useServerApi } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import type { TeamOut, WorkApi } from "../api";
import { TeamPicker, useTeam, useWorkKeys, WorkNav } from "../nav";
import type { ProjectOut } from "../planning/api";
import type { TeamFile } from "../settings/team-file";

import { Recurring } from "./recurring";
import { TemplateEditor, TemplateRow } from "./template-editor";
import { copyName, moved, type Template } from "./template-file";

import "../settings/settings.css";
import "./templates.css";

/** Which template the editor is open on: a new one, or one by index. */
type Editing = { readonly kind: "new" } | { readonly kind: "edit"; readonly index: number };

/** The templates once the one being edited is saved. */
function withSaved(
  templates: readonly Template[],
  editing: Editing,
  template: Template,
): readonly Template[] {
  return editing.kind === "new"
    ? [...templates, template]
    : templates.map((each, index) => (index === editing.index ? template : each));
}

/** The names the template being edited must not take. */
function otherNames(templates: readonly Template[], editing: Editing): readonly string[] {
  return templates
    .filter((_, index) => editing.kind === "new" || index !== editing.index)
    .map((each) => each.name);
}

function AddButton({
  disabled,
  error,
  onAdd,
}: {
  readonly disabled: boolean;
  readonly error: string | undefined;
  readonly onAdd: () => void;
}) {
  const t = useT();
  return (
    <>
      <button type="button" className="vl-filled-button" disabled={disabled} onClick={onAdd}>
        {t("tpl-new")}
      </button>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}

function Templates({
  api,
  team,
  file,
  projects,
  onSaved,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly file: TeamFile;
  readonly projects: readonly ProjectOut[];
  readonly onSaved: (message: string) => void;
}) {
  const t = useT();
  const templates = file.templates ?? [];
  const [editing, setEditing] = useState<Editing>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const save = (next: readonly Template[], message: string) => {
    setBusy(true);
    setError(undefined);
    api.saveTeamSettings(team.key, { ...file, templates: next }).then(
      () => {
        setBusy(false);
        setEditing(undefined);
        onSaved(message);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  const names = templates.map((each) => each.name);
  const adding = editing?.kind === "new";
  const editingIndex = editing?.kind === "edit" ? editing.index : -1;
  const editor =
    editing === undefined ? undefined : (
      <TemplateEditor
        key={String(editingIndex)}
        file={file}
        before={templates[editingIndex]}
        otherNames={otherNames(templates, editing)}
        projects={projects}
        busy={busy}
        error={error}
        onSave={(template) => {
          save(withSaved(templates, editing, template), t("tpl-saved", { name: template.name }));
        }}
        onCancel={() => {
          setEditing(undefined);
          setError(undefined);
        }}
      />
    );
  return (
    <section className="vl-settings-section" aria-labelledby="vl-tpl-heading">
      <h2 id="vl-tpl-heading">{t("tpl-heading")}</h2>
      <p className="vl-muted">{t("tpl-intro")}</p>
      {templates.length === 0 && !adding && <p className="vl-muted">{t("tpl-empty")}</p>}
      <ul className="vl-tpl-list">
        {templates.map((template, index) =>
          editingIndex === index ? (
            <li key={template.name} className="vl-tpl-row">
              {editor}
            </li>
          ) : (
            <TemplateRow
              key={template.name}
              template={template}
              file={file}
              index={index}
              count={templates.length}
              busy={busy || editing !== undefined}
              onEdit={() => {
                setError(undefined);
                setEditing({ kind: "edit", index });
              }}
              onCopy={() => {
                const copy = { ...template, name: copyName(template.name, names) };
                save(
                  [...templates.slice(0, index + 1), copy, ...templates.slice(index + 1)],
                  t("tpl-saved", { name: copy.name }),
                );
              }}
              onMove={(step) => {
                save(moved(templates, index, step), t("tpl-moved", { name: template.name }));
              }}
              onRemove={() => {
                save(
                  templates.filter((_, other) => other !== index),
                  t("tpl-removed", { name: template.name }),
                );
              }}
            />
          ),
        )}
      </ul>
      {adding ? (
        editor
      ) : (
        <AddButton
          disabled={busy || editing !== undefined}
          error={editing === undefined ? error : undefined}
          onAdd={() => {
            setError(undefined);
            setEditing({ kind: "new" });
          }}
        />
      )}
    </section>
  );
}

function TeamTemplates({ api, team }: { readonly api: WorkApi; readonly team: TeamOut }) {
  const planning = useServerApi("planning");
  const [version, setVersion] = useState(0);
  const [said, setSaid] = useState("");
  useWorkKeys(team.key);
  const loaded = useLoaded(
    async () => {
      const [file, projects] = await Promise.all([
        api.teamSettings(team.key),
        planning === undefined ? Promise.resolve([]) : planning.projects().catch(() => []),
      ]);
      return { file, projects: projects.filter((each) => each.teams.includes(team.key)) };
    },
    [api, planning, team.key, version],
    failureText,
  );
  const done = (message: string) => {
    setSaid(message);
    setVersion((v) => v + 1);
  };
  return (
    <>
      <p role="status" className="vl-visually-hidden">
        {said}
      </p>
      <Ready loaded={loaded}>
        {({ file, projects }) => (
          <Templates
            key={version}
            api={api}
            team={team}
            file={file}
            projects={projects}
            onSaved={done}
          />
        )}
      </Ready>
      {planning !== undefined && (
        <Recurring planning={planning} team={team} version={version} onChanged={done} />
      )}
    </>
  );
}

function Teams({ api }: { readonly api: WorkApi }) {
  const t = useT();
  const loaded = useLoaded(() => api.teams(), [api], failureText);
  return (
    <Ready loaded={loaded}>
      {(teams) => <TeamChoice api={api} teams={teams} empty={t("issues-no-teams")} />}
    </Ready>
  );
}

function TeamChoice({
  api,
  teams,
  empty,
}: {
  readonly api: WorkApi;
  readonly teams: readonly TeamOut[];
  readonly empty: string;
}) {
  const team = useTeam(teams);
  if (team === undefined) return <p className="vl-muted">{empty}</p>;
  return (
    <>
      <div className="vl-work-filters">
        <TeamPicker teams={teams} team={team} reset />
      </div>
      <TeamTemplates key={team.key} api={api} team={team} />
    </>
  );
}

/** The templates and recurring issues screen. */
export function TemplatesPage() {
  const t = useT();
  return (
    <ServerScreen api="work" title={t("tpl-screen")} className="vl-work">
      {(api) => (
        <>
          <WorkNav />
          <Teams api={api} />
        </>
      )}
    </ServerScreen>
  );
}
