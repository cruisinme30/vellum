// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** The simple team settings: name and people, estimates, cycles and Git. */

import { useId, useState } from "react";

import { useT } from "../../i18n/i18n";

import {
  GIT_MOMENTS,
  SCALES,
  START_DAYS,
  type CycleSettings,
  type GitMoment,
  type TeamFile,
  without,
} from "./team-file";

/** Applies one change to the draft. */
export type Edit = (change: (file: TeamFile) => TeamFile) => void;

const list = (text: string) =>
  text
    .split(",")
    .map((part) => part.trim().replace(/^@/, ""))
    .filter((part) => part !== "");

function Text({
  label,
  value,
  onChange,
  hint,
}: {
  readonly label: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly hint?: string;
}) {
  const id = useId();
  // Typed text stays as typed (a trailing comma in a list, say); the file
  // gets the parsed value.
  const [raw, setRaw] = useState(value);
  return (
    <span className="vl-settings-field">
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        value={raw}
        aria-describedby={hint === undefined ? undefined : `${id}-hint`}
        onChange={(event) => {
          setRaw(event.target.value);
          onChange(event.target.value);
        }}
      />
      {hint !== undefined && (
        <small id={`${id}-hint`} className="vl-muted">
          {hint}
        </small>
      )}
    </span>
  );
}

function NumberField({
  label,
  value,
  min,
  max,
  onChange,
}: {
  readonly label: string;
  readonly value: number;
  readonly min: number;
  readonly max: number;
  readonly onChange: (value: number) => void;
}) {
  const id = useId();
  // The box keeps what is typed; the file gets it once it is a whole number
  // in range, and leaving the box puts back the value in force.
  const [raw, setRaw] = useState(String(value));
  return (
    <span className="vl-settings-field">
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        type="number"
        inputMode="numeric"
        min={min}
        max={max}
        value={raw}
        onChange={(event) => {
          setRaw(event.target.value);
          const next = Number(event.target.value);
          if (
            event.target.value !== "" &&
            Number.isInteger(next) &&
            next >= min &&
            next <= max
          ) {
            onChange(next);
          }
        }}
        onBlur={() => {
          setRaw(String(value));
        }}
      />
    </span>
  );
}

function Check({
  label,
  checked,
  onChange,
}: {
  readonly label: string;
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
}) {
  return (
    <label className="vl-settings-check">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => {
          onChange(event.target.checked);
        }}
      />
      {label}
    </label>
  );
}

function Choice({
  label,
  value,
  options,
  onChange,
}: {
  readonly label: string;
  readonly value: string;
  readonly options: readonly (readonly [string, string])[];
  readonly onChange: (value: string) => void;
}) {
  const id = useId();
  return (
    <span className="vl-settings-field">
      <label htmlFor={id}>{label}</label>
      <select
        id={id}
        value={value}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        {options.map(([option, name]) => (
          <option key={option} value={option}>
            {name}
          </option>
        ))}
      </select>
    </span>
  );
}

/** Name, time zone, lead and members. */
export function GeneralSection({
  file,
  edit,
}: {
  readonly file: TeamFile;
  readonly edit: Edit;
}) {
  const t = useT();
  return (
    <section className="vl-settings-section" aria-labelledby="settings-general">
      <h2 id="settings-general">{t("team-settings-general")}</h2>
      <Text
        label={t("team-settings-name")}
        value={file.name}
        onChange={(name) => {
          edit((f) => ({ ...f, name }));
        }}
      />
      <Text
        label={t("team-settings-timezone")}
        value={file.timezone ?? "UTC"}
        hint={t("team-settings-timezone-hint")}
        onChange={(timezone) => {
          edit((f) => ({ ...f, timezone }));
        }}
      />
      <Text
        label={t("team-settings-lead")}
        value={file.lead ?? ""}
        onChange={(lead) => {
          const handle = lead.trim().replace(/^@/, "");
          edit((f) =>
            handle === "" ? without(f, "lead") : { ...f, lead: handle },
          );
        }}
      />
      <Text
        label={t("team-settings-members")}
        value={(file.members ?? []).join(", ")}
        hint={t("team-settings-members-hint")}
        onChange={(members) => {
          edit((f) => ({ ...f, members: list(members) }));
        }}
      />
    </section>
  );
}

/** The estimate scale. */
export function EstimatesSection({
  file,
  edit,
}: {
  readonly file: TeamFile;
  readonly edit: Edit;
}) {
  const t = useT();
  const estimates = file.estimates ?? {};
  return (
    <section
      className="vl-settings-section"
      aria-labelledby="settings-estimates"
    >
      <h2 id="settings-estimates">{t("team-settings-estimates")}</h2>
      <Choice
        label={t("team-settings-scale")}
        value={estimates.scale ?? "fibonacci"}
        options={SCALES.map((scale) => [
          scale,
          t(`team-settings-scale-${scale}`),
        ])}
        onChange={(scale) => {
          edit((f) => ({ ...f, estimates: { ...f.estimates, scale } }));
        }}
      />
      <Check
        label={t("team-settings-allow-zero")}
        checked={estimates.allow_zero ?? false}
        onChange={(allow_zero) => {
          edit((f) => ({ ...f, estimates: { ...f.estimates, allow_zero } }));
        }}
      />
      <NumberField
        label={t("team-settings-unestimated")}
        value={estimates.unestimated_points ?? 1}
        min={0}
        max={100}
        onChange={(unestimated_points) => {
          edit((f) => ({
            ...f,
            estimates: { ...f.estimates, unestimated_points },
          }));
        }}
      />
    </section>
  );
}

const CYCLE_DEFAULTS: CycleSettings = {
  duration_weeks: 2,
  start_day: "monday",
  cooldown_weeks: 0,
  upcoming: 3,
  auto_rollover: true,
  auto_add_started: true,
  auto_add_completed: true,
};

/** Cycles: on or off, length, start day, cooldown and what joins them. */
export function CyclesSection({
  file,
  edit,
}: {
  readonly file: TeamFile;
  readonly edit: Edit;
}) {
  const t = useT();
  const cycles =
    file.cycles === undefined
      ? undefined
      : { ...CYCLE_DEFAULTS, ...file.cycles };
  const set = (change: Partial<CycleSettings>) => {
    edit((f) => ({
      ...f,
      cycles: { ...CYCLE_DEFAULTS, ...f.cycles, ...change },
    }));
  };
  return (
    <section className="vl-settings-section" aria-labelledby="settings-cycles">
      <h2 id="settings-cycles">{t("team-settings-cycles")}</h2>
      <Check
        label={t("team-settings-cycles-on")}
        checked={cycles !== undefined}
        onChange={(on) => {
          edit((f) =>
            on ? { ...f, cycles: CYCLE_DEFAULTS } : without(f, "cycles"),
          );
        }}
      />
      {cycles !== undefined && (
        <div className="vl-settings-grid">
          <NumberField
            label={t("team-settings-cycle-weeks")}
            value={cycles.duration_weeks}
            min={1}
            max={8}
            onChange={(duration_weeks) => {
              set({ duration_weeks });
            }}
          />
          <Choice
            label={t("team-settings-cycle-start")}
            value={cycles.start_day ?? "monday"}
            options={START_DAYS.map((day) => [
              day,
              t(`team-settings-day-${day}`),
            ])}
            onChange={(start_day) => {
              set({ start_day });
            }}
          />
          <NumberField
            label={t("team-settings-cycle-cooldown")}
            value={cycles.cooldown_weeks ?? 0}
            min={0}
            max={4}
            onChange={(cooldown_weeks) => {
              set({ cooldown_weeks });
            }}
          />
          <NumberField
            label={t("team-settings-cycle-upcoming")}
            value={cycles.upcoming ?? 3}
            min={1}
            max={15}
            onChange={(upcoming) => {
              set({ upcoming });
            }}
          />
          <Check
            label={t("team-settings-cycle-rollover")}
            checked={cycles.auto_rollover ?? true}
            onChange={(auto_rollover) => {
              set({ auto_rollover });
            }}
          />
          <Check
            label={t("team-settings-cycle-add-started")}
            checked={cycles.auto_add_started ?? true}
            onChange={(auto_add_started) => {
              set({ auto_add_started });
            }}
          />
          <Check
            label={t("team-settings-cycle-add-completed")}
            checked={cycles.auto_add_completed ?? true}
            onChange={(auto_add_completed) => {
              set({ auto_add_completed });
            }}
          />
        </div>
      )}
    </section>
  );
}

/** Which status each Git moment moves an issue to. */
export function GitSection({
  file,
  edit,
}: {
  readonly file: TeamFile;
  readonly edit: Edit;
}) {
  const t = useT();
  const statuses: [string, string][] = [
    ["", t("team-settings-git-stay")],
    ...file.workflow.statuses.map((status): [string, string] => [
      status.key,
      status.name,
    ]),
  ];
  const set = (moment: GitMoment, status: string) => {
    edit((f) => {
      const git = without({ ...f.git }, moment);
      return { ...f, git: status === "" ? git : { ...git, [moment]: status } };
    });
  };
  return (
    <section className="vl-settings-section" aria-labelledby="settings-git">
      <h2 id="settings-git">{t("team-settings-git")}</h2>
      <p className="vl-muted">
        {file.git === undefined
          ? t("team-settings-git-default")
          : t("team-settings-git-own")}
      </p>
      <div className="vl-settings-grid">
        {GIT_MOMENTS.map((moment) => (
          <Choice
            key={moment}
            label={t(`team-settings-git-${moment}`)}
            value={file.git?.[moment] ?? ""}
            options={statuses}
            onChange={(status) => {
              set(moment, status);
            }}
          />
        ))}
      </div>
    </section>
  );
}

export { Check, Choice, NumberField, Text };
