// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Saved and shared views, and the team's statuses by category (R-WK-3, R-WK-4). */

import { useId, useState, type ComponentProps } from "react";

import { useSession } from "../api/session";
import { failureText } from "../data/failure";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { TeamOut, ViewOut, WorkApi } from "./api";
import { fieldText } from "./form";
import { CATEGORIES, VISIBILITIES, viewInput, type Settings } from "./view-settings";

export function SavedViews({
  api,
  team,
  settings,
  apply,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly settings: Settings;
  readonly apply: (view?: ViewOut) => void;
}) {
  const t = useT();
  const id = useId();
  const me = useSession()?.status.user?.handle ?? undefined;
  const [version, setVersion] = useState(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(() => api.views(team.key), [api, team.key, version], failureText);
  const views = loaded.state === "ready" ? loaded.value : [];
  const current = views.find((view) => view.id === settings.view);
  const run = (work: Promise<unknown>, after?: () => void) => {
    setError(undefined);
    work.then(
      () => {
        after?.();
        setVersion((v) => v + 1);
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  const save: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const name = fieldText(data, "name");
    if (name === "") return;
    const visibility = fieldText(data, "visibility") || "private";
    setError(undefined);
    api.createView(viewInput(team, settings, name, visibility)).then(
      (view) => {
        setSaving(false);
        setVersion((v) => v + 1);
        apply(view);
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  return (
    <div className="vl-saved-views">
      <span className="vl-work-field">
        <label htmlFor={id}>{t("work-views")}</label>
        <select
          id={id}
          value={current?.id ?? ""}
          onChange={(event) => {
            apply(views.find((view) => view.id === event.target.value));
          }}
        >
          <option value="">{t("work-views-none")}</option>
          {views.map((view) => (
            <option key={view.id} value={view.id}>
              {view.visibility === "private"
                ? view.name
                : t("work-views-shared", { name: view.name, owner: view.owner })}
            </option>
          ))}
        </select>
      </span>
      {current !== undefined && current.owner === me && (
        <>
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              run(
                api.updateView(
                  current.id,
                  viewInput(team, settings, current.name, current.visibility),
                ),
              );
            }}
          >
            {t("work-views-update")}
          </button>
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              run(api.deleteView(current.id), () => {
                apply();
              });
            }}
          >
            {t("work-views-delete")}
          </button>
        </>
      )}
      {saving ? (
        <form className="vl-work-add" onSubmit={save}>
          <label>
            {t("work-views-name")}
            <input name="name" autoComplete="off" required maxLength={120} />
          </label>
          <label>
            {t("work-views-visibility")}
            <select name="visibility" defaultValue="private">
              {VISIBILITIES.map((visibility) => (
                <option key={visibility} value={visibility}>
                  {t(`work-visibility-${visibility}`, { team: team.name })}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" className="vl-filled-button">
            {t("work-views-save")}
          </button>
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setSaving(false);
            }}
          >
            {t("work-cancel")}
          </button>
        </form>
      ) : (
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            setSaving(true);
          }}
        >
          {t("work-views-save-as")}
        </button>
      )}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

export function Statuses({ team }: { readonly team: TeamOut }) {
  const t = useT();
  return (
    <details className="vl-work-statuses">
      <summary>{t("work-statuses", { team: team.name })}</summary>
      <dl>
        {CATEGORIES.map((category) => {
          const statuses = team.statuses.filter((status) => status.category === category);
          if (statuses.length === 0) return;
          return (
            <div key={category}>
              <dt>{t(`work-category-${category}`)}</dt>
              {statuses.map((status) => (
                <dd key={status.key}>
                  {status.name}
                  {status.key === team.default_status && (
                    <span className="vl-work-chip">{t("work-status-default")}</span>
                  )}
                </dd>
              ))}
            </div>
          );
        })}
      </dl>
      <p className="vl-muted">{t("work-statuses-hint")}</p>
    </details>
  );
}
