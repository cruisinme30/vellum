// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps, type ReactNode } from "react";

import { failureText } from "../../data/failure";
import { useT } from "../../i18n/i18n";

import "./planning.css";

/** A form's submit: busy while the promise runs, its failure shown after. */
export function useSubmit<T>(
  run: (form: FormData) => Promise<T> | undefined,
  done: (value: T) => void,
) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const onSubmit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const pending = run(new FormData(form));
    if (pending === undefined) return;
    setBusy(true);
    setError(undefined);
    pending.then(
      (value) => {
        setBusy(false);
        form.reset();
        done(value);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  return { busy, error, onSubmit };
}

/** A form field's text, trimmed; empty when absent. */
export function text(form: FormData, name: string): string {
  const value = form.get(name);
  return typeof value === "string" ? value.trim() : "";
}

/** A failure line under a form. */
export function Failure({ error }: { readonly error: string | undefined }) {
  if (error === undefined) return;
  return (
    <p className="vl-field-error" role="alert">
      {error}
    </p>
  );
}

/** A labelled control in a planning form. */
export function Field({
  label,
  children,
}: {
  readonly label: string;
  readonly children: ReactNode;
}) {
  return (
    <label className="vl-plan-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

const HEALTH = ["on-track", "at-risk", "off-track"] as const;
const NO_HEALTH = "none";

const PRIORITY_NAMES = ["none", "urgent", "high", "normal", "low"] as const;

/** The message id of a priority number, 0 (none) to 4 (low). */
export function priorityId(priority: number): string {
  const name: string = PRIORITY_NAMES[priority] ?? PRIORITY_NAMES[0];
  return `plan-priority-${name}`;
}

/** The health values a person can pick, in order. */
export const HEALTHS: readonly string[] = HEALTH;

/** A project's or initiative's health, as a coloured chip with text. */
export function HealthChip({ health }: { readonly health: string | null | undefined }) {
  const t = useT();
  const known: string = HEALTH.find((h) => h === health) ?? NO_HEALTH;
  return <span className={`vl-health vl-health-${known}`}>{t(`plan-health-${known}`)}</span>;
}

/** A labelled progress bar: `percent` complete. */
export function Progress({ percent, label }: { readonly percent: number; readonly label: string }) {
  const value = Math.max(0, Math.min(100, Math.round(percent)));
  return (
    <span className="vl-progress">
      <progress max={100} value={value} aria-label={label} />
      <span>{value}%</span>
    </span>
  );
}

const DAY = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
  timeZone: "UTC",
});

/** A `YYYY-MM-DD` date, written for people. */
export function day(iso: string | null | undefined): string {
  if (iso == undefined || iso === "") return "";
  const parsed = new Date(`${iso}T00:00:00Z`);
  return Number.isNaN(parsed.getTime()) ? iso : DAY.format(parsed);
}

/** Today, `YYYY-MM-DD`, shifted by `days`. */
export function isoDay(days = 0, from = new Date()): string {
  const shifted = new Date(from.getTime() + days * 86_400_000);
  return shifted.toISOString().slice(0, 10);
}
