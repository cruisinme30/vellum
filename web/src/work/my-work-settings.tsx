// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** My Work settings: time zone, start of day and digest time (ADR 0042). */

import { useState, type ComponentProps } from "react";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { CopyDiagnostic } from "../shell/copy-diagnostic";

import type { MyWorkSettingsView, WorkApi } from "./api";

// The API reads null as "none": the server's zone, no digest.
// eslint-disable-next-line unicorn/no-null
const NONE = null;

/** The browser's IANA time zone, if it reports one. */
function browserZone(): string | undefined {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return undefined;
  }
}

/** Every zone the browser knows, for the picker. */
function knownZones(): readonly string[] {
  try {
    return Intl.supportedValuesOf("timeZone");
  } catch {
    return [];
  }
}

/** Time zone, start of day and digest time. */
function SettingsForm({
  api,
  settings,
  onSaved,
}: {
  readonly api: WorkApi;
  readonly settings: MyWorkSettingsView;
  readonly onSaved: () => void;
}) {
  const t = useT();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const here = browserZone();
  const save: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const text = (name: string) => {
      const value = data.get(name);
      return typeof value === "string" ? value.trim() : "";
    };
    setError(undefined);
    setSaved(false);
    api
      .saveSettings({
        time_zone: text("time_zone") === "" ? NONE : text("time_zone"),
        day_starts_at: text("day_starts_at"),
        digest_at: text("digest_at") === "" ? NONE : text("digest_at"),
      })
      .then(
        () => {
          setSaved(true);
          onSaved();
        },
        (error_: unknown) => {
          setError(failureText(error_));
        },
      );
  };
  return (
    <form className="vl-work-settings" onSubmit={save}>
      <p className="vl-muted">
        {t("my-work-settings-zone-now", { zone: settings.effective_time_zone })}
      </p>
      <label>
        {t("my-work-settings-zone")}
        <input
          name="time_zone"
          list="vl-time-zones"
          autoComplete="off"
          defaultValue={settings.time_zone ?? here ?? ""}
          placeholder={t("my-work-settings-zone-server")}
        />
      </label>
      <datalist id="vl-time-zones">
        {knownZones().map((zone) => (
          <option key={zone} value={zone} />
        ))}
      </datalist>
      <label>
        {t("my-work-settings-day")}
        <input
          name="day_starts_at"
          type="time"
          required
          defaultValue={settings.day_starts_at}
        />
      </label>
      <label>
        {t("my-work-settings-digest")}
        <input
          name="digest_at"
          type="time"
          defaultValue={settings.digest_at ?? ""}
        />
      </label>
      <p className="vl-muted">{t("my-work-settings-digest-hint")}</p>
      <button type="submit" className="vl-filled-button">
        {t("my-work-settings-save")}
      </button>
      {saved && <p role="status">{t("my-work-settings-saved")}</p>}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
          <CopyDiagnostic message={error} />
        </p>
      )}
    </form>
  );
}

/** The time zone and reminders panel. */
export function Settings({
  api,
  onSaved,
}: {
  readonly api: WorkApi;
  readonly onSaved: () => void;
}) {
  const t = useT();
  const loaded = useLoaded(() => api.settings(), [api], failureText);
  return (
    <details className="vl-work-settings-panel">
      <summary>{t("my-work-settings")}</summary>
      <Ready loaded={loaded}>
        {(settings) => (
          <SettingsForm api={api} settings={settings} onSaved={onSaved} />
        )}
      </Ready>
    </details>
  );
}
