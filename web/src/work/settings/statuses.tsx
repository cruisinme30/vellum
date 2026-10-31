// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Statuses (R-WK-3): custom statuses in the fixed categories, one default.
 * Removing a status that still has issues asks where they go; the moves
 * run when the settings are saved, before the status disappears.
 */

import { useId, useState, type ComponentProps } from "react";

import { failureText } from "../../data/failure";
import { useT } from "../../i18n/i18n";

import type { Edit } from "./sections";
import {
  CATEGORIES,
  freshKey,
  moveWithinCategory,
  pruneGit,
  type Category,
  type Status,
  type TeamFile,
} from "./team-file";

/** Issues still in a removed status, and where they go. */
export type Moves = Readonly<Record<string, string>>;

function Removal({
  status,
  file,
  count,
  onRemove,
  onCancel,
}: {
  readonly status: Status;
  readonly file: TeamFile;
  readonly count: number;
  readonly onRemove: (moveTo?: string) => void;
  readonly onCancel: () => void;
}) {
  const t = useT();
  const id = useId();
  const others = file.workflow.statuses.filter(
    (each) => each.key !== status.key && each.category !== "triage",
  );
  const [target, setTarget] = useState(
    others.find((each) => each.category === status.category)?.key ??
      others[0]?.key ??
      "",
  );
  return (
    <div
      className="vl-settings-removal"
      role="group"
      aria-label={t("team-settings-remove-status", { name: status.name })}
    >
      <label htmlFor={id}>
        {t("team-settings-move-issues", { count, name: status.name })}
      </label>
      <select
        id={id}
        value={target}
        onChange={(event) => {
          setTarget(event.target.value);
        }}
      >
        {others.map((each) => (
          <option key={each.key} value={each.key}>
            {each.name}
          </option>
        ))}
      </select>
      <button
        type="button"
        className="vl-filled-button"
        onClick={() => {
          onRemove(target);
        }}
      >
        {t("team-settings-remove-confirm")}
      </button>
      <button type="button" className="vl-text-button" onClick={onCancel}>
        {t("work-cancel")}
      </button>
    </div>
  );
}

function StatusRow({
  status,
  file,
  edit,
  countIssues,
  onMove,
}: {
  readonly status: Status;
  readonly file: TeamFile;
  readonly edit: Edit;
  readonly countIssues: (status: string) => Promise<number>;
  readonly onMove: (from: string, to: string) => void;
}) {
  const t = useT();
  const [removing, setRemoving] = useState<number>();
  const [error, setError] = useState<string>();
  const canDefault =
    status.category === "backlog" || status.category === "unstarted";
  const peers = file.workflow.statuses.filter(
    (each) => each.category === status.category,
  );
  const rename = (name: string) => {
    edit((f) => ({
      ...f,
      workflow: {
        ...f.workflow,
        statuses: f.workflow.statuses.map((each) =>
          each.key === status.key ? { ...each, name } : each,
        ),
      },
    }));
  };
  const remove = (moveTo?: string) => {
    setRemoving(undefined);
    if (moveTo !== undefined) onMove(status.key, moveTo);
    edit((f) =>
      pruneGit({
        ...f,
        workflow: {
          ...f.workflow,
          statuses: f.workflow.statuses.filter(
            (each) => each.key !== status.key,
          ),
        },
      }),
    );
  };
  const move = (step: -1 | 1) => {
    edit((f) => ({
      ...f,
      workflow: {
        ...f.workflow,
        statuses: moveWithinCategory(f.workflow.statuses, status.key, step),
      },
    }));
  };
  const index = peers.findIndex((each) => each.key === status.key);
  return (
    <li className="vl-settings-status">
      <input
        aria-label={t("team-settings-status-name", { key: status.key })}
        value={status.name}
        onChange={(event) => {
          rename(event.target.value);
        }}
      />
      <label className="vl-settings-check">
        <input
          type="radio"
          name="default-status"
          checked={file.workflow.default === status.key}
          disabled={!canDefault}
          onChange={() => {
            edit((f) => ({
              ...f,
              workflow: { ...f.workflow, default: status.key },
            }));
          }}
        />
        {t("team-settings-default")}
      </label>
      <button
        type="button"
        className="vl-text-button"
        disabled={index <= 0}
        aria-label={t("team-settings-up", { name: status.name })}
        onClick={() => {
          move(-1);
        }}
      >
        ↑
      </button>
      <button
        type="button"
        className="vl-text-button"
        disabled={index === peers.length - 1}
        aria-label={t("team-settings-down", { name: status.name })}
        onClick={() => {
          move(1);
        }}
      >
        ↓
      </button>
      <button
        type="button"
        className="vl-text-button"
        disabled={file.workflow.default === status.key || peers.length === 1}
        aria-label={t("team-settings-remove-status", { name: status.name })}
        onClick={() => {
          setError(undefined);
          countIssues(status.key).then(
            (count) => {
              if (count === 0) remove();
              else setRemoving(count);
            },
            (error_: unknown) => {
              setError(failureText(error_));
            },
          );
        }}
      >
        {t("team-settings-remove")}
      </button>
      {removing !== undefined && (
        <Removal
          status={status}
          file={file}
          count={removing}
          onRemove={remove}
          onCancel={() => {
            setRemoving(undefined);
          }}
        />
      )}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </li>
  );
}

function AddStatus({
  category,
  edit,
}: {
  readonly category: Category;
  readonly edit: Edit;
}) {
  const t = useT();
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const name = new FormData(form).get("name");
    if (typeof name !== "string" || name.trim() === "") return;
    edit((f) => {
      const statuses = [...f.workflow.statuses];
      const key = freshKey(
        name,
        statuses.map((each) => each.key),
      );
      // After the last status of the same category, so board order holds.
      const last = statuses.findLastIndex((each) => each.category === category);
      statuses.splice(last + 1, 0, { key, name: name.trim(), category });
      return { ...f, workflow: { ...f.workflow, statuses } };
    });
    form.reset();
  };
  return (
    <form className="vl-settings-add" onSubmit={submit}>
      <input
        name="name"
        autoComplete="off"
        aria-label={t("team-settings-add-status", {
          category: t(`work-category-${category}`),
        })}
        placeholder={t("team-settings-add-status-hint")}
      />
      <button type="submit" className="vl-text-button">
        {t("team-settings-add")}
      </button>
    </form>
  );
}

/** The statuses, by category. */
export function StatusesSection({
  file,
  edit,
  countIssues,
  onMove,
}: {
  readonly file: TeamFile;
  readonly edit: Edit;
  readonly countIssues: (status: string) => Promise<number>;
  readonly onMove: (from: string, to: string) => void;
}) {
  const t = useT();
  return (
    <section
      className="vl-settings-section"
      aria-labelledby="settings-statuses"
    >
      <h2 id="settings-statuses">{t("team-settings-statuses")}</h2>
      <p className="vl-muted">{t("team-settings-statuses-hint")}</p>
      {CATEGORIES.map((category) => {
        const statuses = file.workflow.statuses.filter(
          (status) => status.category === category,
        );
        // One triage and one duplicate status at most; the engine keeps them
        // fixed.
        const fixed = category === "triage" || category === "duplicate";
        if (fixed && statuses.length === 0) return;
        return (
          <fieldset key={category} className="vl-settings-category">
            <legend>{t(`work-category-${category}`)}</legend>
            <ul>
              {statuses.map((status) => (
                <StatusRow
                  key={status.key}
                  status={status}
                  file={file}
                  edit={edit}
                  countIssues={countIssues}
                  onMove={onMove}
                />
              ))}
            </ul>
            {!fixed && <AddStatus category={category} edit={edit} />}
          </fieldset>
        );
      })}
    </section>
  );
}
