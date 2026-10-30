// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps, type ReactNode } from "react";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";

import {
  formText,
  isConflict,
  type AdminConsoleApi,
  type AdminSettings,
  type Versioned,
} from "./api";
import { AdminScreen } from "./context";

/** One editable setting: where it lives, and how the form shows it. */
interface Field {
  readonly group: "telemetry" | "observability" | "backups" | "security" | "appearance";
  readonly key: string;
  readonly kind: "number" | "text" | "toggle" | "color" | "select";
  /** Smallest allowed number. */
  readonly min?: number;
  /** A select's values. */
  readonly options?: readonly string[];
}

/** The theme ids the server accepts (vellum-admin `EXPORT_THEMES`, `SITE_THEMES`). */
export const EXPORT_THEMES = ["light", "high-contrast", "sepia"] as const;
export const SITE_THEMES = ["auto", "light", "dark", "high-contrast", "sepia", "dim"] as const;

const FIELDS: readonly Field[] = [
  { group: "appearance", key: "seed", kind: "color" },
  { group: "appearance", key: "export_theme", kind: "select", options: EXPORT_THEMES },
  { group: "appearance", key: "site_theme", kind: "select", options: SITE_THEMES },
  { group: "backups", key: "enabled", kind: "toggle" },
  { group: "backups", key: "interval_hours", kind: "number", min: 1 },
  { group: "backups", key: "keep", kind: "number", min: 1 },
  { group: "backups", key: "destination", kind: "text" },
  { group: "backups", key: "drill_interval_days", kind: "number", min: 1 },
  { group: "security", key: "fresh_auth_minutes", kind: "number", min: 1 },
  { group: "security", key: "session_hours", kind: "number", min: 1 },
  { group: "observability", key: "slow_request_ms", kind: "number", min: 1 },
  { group: "observability", key: "trace_buffer_capacity", kind: "number", min: 1 },
  { group: "observability", key: "trace_buffer_mib", kind: "number", min: 1 },
  { group: "telemetry", key: "enabled", kind: "toggle" },
  { group: "telemetry", key: "endpoint", kind: "text" },
  { group: "telemetry", key: "interval_hours", kind: "number", min: 1 },
];

const GROUPS = ["appearance", "backups", "security", "observability", "telemetry"] as const;

type Group = Readonly<Record<string, unknown>>;

const groupOf = (settings: AdminSettings, group: Field["group"]): Group =>
  (settings[group] ?? {}) as Group;

/** The form's text for one field. */
function shownText(settings: AdminSettings, field: Field): string {
  const value = groupOf(settings, field.group)[field.key];
  return typeof value === "number" || typeof value === "string" ? String(value) : "";
}

/** Marks a field the form leaves as it was. */
const KEEP = Symbol("keep");

/** One field's value from the form. */
function fieldValue(field: Field, data: FormData, name: string): unknown {
  const text = formText(data, name);
  switch (field.kind) {
    case "toggle": {
      return data.get(name) === "on";
    }
    case "number": {
      // Left empty: the server keeps its default.
      return text === "" ? KEEP : Number(text);
    }
    case "text": {
      return field.key === "endpoint" && text === "" ? undefined : text;
    }
    case "color":
    case "select": {
      return text === "" ? KEEP : text;
    }
  }
}

/**
 * The settings from the form. Fields the form does not show (route
 * budgets, the installation ID) are kept as they were read.
 */
export function settingsFromForm(base: AdminSettings, data: FormData): AdminSettings {
  const next: Record<string, unknown> = {
    ...base,
    organisation: formText(data, "organisation"),
  };
  for (const group of GROUPS) {
    const values: Record<string, unknown> = { ...groupOf(base, group) };
    for (const field of FIELDS.filter((each) => each.group === group)) {
      const name = `${group}.${field.key}`;
      const value = fieldValue(field, data, name);
      if (value !== KEEP) values[field.key] = value;
    }
    next[group] = values;
  }
  return next;
}

function FieldInput({
  settings,
  field,
}: {
  readonly settings: AdminSettings;
  readonly field: Field;
}) {
  const t = useT();
  const name = `${field.group}.${field.key}`;
  const label = t(`admin-setting-${field.group}-${field.key.replaceAll("_", "-")}`);
  if (field.kind === "toggle") {
    return (
      <label className="vl-admin-toggle">
        <input
          type="checkbox"
          name={name}
          defaultChecked={groupOf(settings, field.group)[field.key] === true}
        />
        {label}
      </label>
    );
  }
  if (field.kind === "select") {
    return (
      <label>
        {label}
        <select name={name} defaultValue={shownText(settings, field)}>
          {(field.options ?? []).map((option) => (
            <option key={option} value={option}>
              {t(`admin-theme-${option}`)}
            </option>
          ))}
        </select>
      </label>
    );
  }
  if (field.kind === "color") {
    return (
      <label>
        {label}
        <input name={name} type="color" defaultValue={shownText(settings, field) || "#4f5b92"} />
      </label>
    );
  }
  return (
    <label>
      {label}
      <input
        name={name}
        type={field.kind === "number" ? "number" : "text"}
        inputMode={field.kind === "number" ? "numeric" : undefined}
        min={field.min}
        defaultValue={shownText(settings, field)}
      />
    </label>
  );
}

function Fieldset({ title, children }: { readonly title: string; readonly children: ReactNode }) {
  return (
    <fieldset className="vl-admin-fieldset">
      <legend>{title}</legend>
      {children}
    </fieldset>
  );
}

function SettingsForm({
  api,
  initial,
  onReload,
}: {
  readonly api: AdminConsoleApi;
  readonly initial: Versioned;
  readonly onReload: () => void;
}) {
  const t = useT();
  const [current, setCurrent] = useState(initial);
  const [saving, setSaving] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string; conflict?: boolean }>();

  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const settings = settingsFromForm(current.settings, new FormData(event.currentTarget));
    setSaving(true);
    setResult(undefined);
    api
      .saveSettings(current.version, settings)
      .then(
        (saved) => {
          setCurrent(saved);
          setResult({ ok: true, message: t("admin-settings-saved") });
        },
        (error: unknown) => {
          setResult(
            isConflict(error)
              ? { ok: false, message: t("admin-settings-conflict"), conflict: true }
              : { ok: false, message: failureText(error) },
          );
        },
      )
      .finally(() => {
        setSaving(false);
      });
  };

  return (
    <form className="vl-admin-form" onSubmit={submit}>
      <label>
        {t("admin-setting-organisation")}
        <input name="organisation" defaultValue={current.settings.organisation ?? ""} />
      </label>
      {GROUPS.map((group) => (
        <Fieldset key={group} title={t(`admin-settings-${group}`)}>
          {FIELDS.filter((field) => field.group === group).map((field) => (
            <FieldInput key={field.key} settings={current.settings} field={field} />
          ))}
        </Fieldset>
      ))}
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={saving}>
          {saving ? t("admin-saving") : t("admin-save")}
        </button>
        <div role="status">{result?.ok === true && <p>{result.message}</p>}</div>
      </div>
      {result?.ok === false && (
        <div className="vl-field-error" role="alert">
          <p>{result.message}</p>
          <CopyDiagnostic message={result.message} />
          {result.conflict === true && (
            <button type="button" className="vl-text-button" onClick={onReload}>
              {t("admin-reload")}
            </button>
          )}
        </div>
      )}
    </form>
  );
}

function SettingsLoader({ api }: { readonly api: AdminConsoleApi }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const loaded = useLoaded(() => api.settings(), [api, reload], failureText);
  if (loaded.state === "loading") {
    return <p className="vl-muted">{t("admin-loading")}</p>;
  }
  if (loaded.state === "failed") {
    return (
      <p className="vl-field-error" role="alert">
        {loaded.message}
        <CopyDiagnostic message={loaded.message} />
      </p>
    );
  }
  return (
    <SettingsForm
      key={`${String(reload)}:${String(loaded.value.version)}`}
      api={api}
      initial={loaded.value}
      onReload={() => {
        setReload((n) => n + 1);
      }}
    />
  );
}

/** Admin → Settings: appearance, backups, sessions, observability and telemetry. */
export function SettingsPage() {
  const t = useT();
  return (
    <AdminScreen
      help="admin.settings"
      title={t("admin-settings")}
      intro={t("admin-settings-intro")}
    >
      {(api) => <SettingsLoader api={api} />}
    </AdminScreen>
  );
}
