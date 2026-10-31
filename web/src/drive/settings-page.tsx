// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type SyntheticEvent } from "react";
import { Link, useParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useT } from "../i18n/i18n";
import { CopyDiagnostic } from "../shell/copy-diagnostic";

import type { DriveApi, SettingsView, SpaceView } from "./api";
import { DriveFrame } from "./drive-page";
import { loadSpaces, useReloadable } from "./loading";
import { spacePath } from "./paths";
import { UsageMeter } from "./usage";

const GB = 1024 ** 3;

/** The form's text fields; empty means "no limit" or "forever". */
interface Draft {
  readonly name: string;
  readonly quotaGb: string;
  readonly trashDays: string;
  readonly keepAllDays: string;
  readonly dailyDays: string;
  readonly weeklyWeeks: string;
}

export function draftOf(space: SpaceView): Draft {
  const s = space.settings;
  return {
    name: space.name,
    quotaGb:
      s.quota_bytes === undefined || s.quota_bytes === null
        ? ""
        : String(s.quota_bytes / GB),
    trashDays: String(s.trash_days),
    keepAllDays: String(s.keep_all_days),
    dailyDays: String(s.daily_days),
    weeklyWeeks:
      s.weekly_weeks === undefined || s.weekly_weeks === null
        ? ""
        : String(s.weekly_weeks),
  };
}

const whole = (text: string) => {
  const value = Number(text.trim());
  return text.trim() !== "" && Number.isInteger(value) && value >= 0
    ? value
    : undefined;
};

/**
 * An optional field: empty is "none", else it must pass `parse`; `false` when
 * it does not.
 */
const optional = (text: string, parse: (text: string) => number | undefined) =>
  text.trim() === "" ? undefined : (parse(text) ?? false);

const positive = (text: string) => {
  const value = Number(text.trim());
  return Number.isFinite(value) && value > 0 ? value : undefined;
};

/**
 * The settings a draft says, or the name of the first field that is not a
 * valid number. Quota is in GB and may have decimals; the rest are whole.
 */
export function settingsOf(
  draft: Draft,
): SettingsView | { readonly invalid: keyof Draft } {
  const quota = optional(draft.quotaGb, positive);
  if (quota === false) return { invalid: "quotaGb" };
  const trashDays = whole(draft.trashDays);
  if (trashDays === undefined || trashDays < 1) return { invalid: "trashDays" };
  const keepAllDays = whole(draft.keepAllDays);
  if (keepAllDays === undefined) return { invalid: "keepAllDays" };
  const dailyDays = whole(draft.dailyDays);
  if (dailyDays === undefined) return { invalid: "dailyDays" };
  const weekly = optional(draft.weeklyWeeks, whole);
  if (weekly === false) return { invalid: "weeklyWeeks" };
  return {
    ...(quota === undefined ? {} : { quota_bytes: Math.round(quota * GB) }),
    trash_days: trashDays,
    keep_all_days: keepAllDays,
    daily_days: dailyDays,
    ...(weekly === undefined ? {} : { weekly_weeks: weekly }),
  };
}

const FIELDS: readonly { key: keyof Draft; label: string; hint?: string }[] = [
  { key: "name", label: "drive-settings-name" },
  {
    key: "quotaGb",
    label: "drive-settings-quota",
    hint: "drive-settings-quota-hint",
  },
  { key: "trashDays", label: "drive-settings-trash-days" },
  { key: "keepAllDays", label: "drive-settings-keep-all" },
  { key: "dailyDays", label: "drive-settings-daily" },
  {
    key: "weeklyWeeks",
    label: "drive-settings-weekly",
    hint: "drive-settings-weekly-hint",
  },
];

function Field({
  field,
  value,
  invalid,
  onChange,
}: {
  readonly field: (typeof FIELDS)[number];
  readonly value: string;
  readonly invalid: boolean;
  readonly onChange: (value: string) => void;
}) {
  const t = useT();
  const id = useId();
  const hintId = `${id}-hint`;
  return (
    <div className="vl-drive-field">
      <label htmlFor={id}>{t(field.label)}</label>
      <input
        id={id}
        inputMode={field.key === "name" ? "text" : "decimal"}
        value={value}
        aria-invalid={invalid}
        aria-describedby={field.hint === undefined ? undefined : hintId}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      />
      {field.hint !== undefined && (
        <small id={hintId} className="vl-muted">
          {t(field.hint)}
        </small>
      )}
    </div>
  );
}

function SettingsForm({
  api,
  space,
  onSaved,
}: {
  readonly api: DriveApi;
  readonly space: SpaceView;
  readonly onSaved: () => void;
}) {
  const t = useT();
  const [draft, setDraft] = useState(() => draftOf(space));
  const [invalid, setInvalid] = useState<keyof Draft>();
  const [notice, setNotice] = useState<string>();
  const [problem, setProblem] = useState<string>();

  const run = async (action: () => Promise<string>) => {
    setProblem(undefined);
    try {
      setNotice(await action());
    } catch (error) {
      setProblem(failureText(error));
    }
  };
  const save = async (event: SyntheticEvent) => {
    event.preventDefault();
    const settings = settingsOf(draft);
    if ("invalid" in settings) {
      setInvalid(settings.invalid);
      setProblem(t("drive-settings-invalid"));
      return;
    }
    setInvalid(undefined);
    const name = draft.name.trim();
    await run(async () => {
      await api.updateSpace(
        space.id,
        name === "" || name === space.name ? undefined : name,
        settings,
      );
      onSaved();
      return t("drive-settings-saved");
    });
  };

  return (
    <form className="vl-drive-settings" onSubmit={(event) => void save(event)}>
      {FIELDS.map((field) => (
        <Field
          key={field.key}
          field={field}
          value={draft[field.key]}
          invalid={invalid === field.key}
          onChange={(value) => {
            setDraft((current) => ({ ...current, [field.key]: value }));
          }}
        />
      ))}
      <p className="vl-muted">{t("drive-settings-retention-hint")}</p>
      <p role="status" className="vl-drive-notice">
        {notice}
      </p>
      {problem !== undefined && (
        <p className="vl-field-error" role="alert">
          {problem}
          <CopyDiagnostic message={problem} />
        </p>
      )}
      <div className="vl-drive-dialog-actions">
        <button
          type="button"
          className="vl-text-button"
          onClick={() =>
            void run(async () =>
              t("drive-pruned", { count: await api.pruneVersions(space.id) }),
            )
          }
        >
          {t("drive-prune")}
        </button>
        <button type="submit" className="vl-filled-button">
          {t("drive-settings-save")}
        </button>
      </div>
    </form>
  );
}

function SpaceSettings({
  api,
  spaceId,
}: {
  readonly api: DriveApi;
  readonly spaceId: string;
}) {
  const t = useT();
  const { loaded, reload } = useReloadable(async () => {
    const spaces = await loadSpaces(api);
    return spaces.find((space) => space.id === spaceId);
  }, [api, spaceId]);
  return (
    <Ready loaded={loaded}>
      {(space) =>
        space === undefined ? (
          <p className="vl-muted">{t("drive-space-missing")}</p>
        ) : (
          <>
            <h2>
              <Link to={spacePath(space.id)}>{space.name}</Link> ·{" "}
              {t("drive-settings")}
            </h2>
            <UsageMeter api={api} space={space} />
            <SettingsForm api={api} space={space} onSaved={reload} />
          </>
        )
      }
    </Ready>
  );
}

/** A space's quota, trash and version retention settings (R-DR-10). */
export function DriveSettingsPage() {
  const { spaceId = "" } = useParams();
  return (
    <DriveFrame>
      {(api) => <SpaceSettings api={api} spaceId={spaceId} />}
    </DriveFrame>
  );
}
