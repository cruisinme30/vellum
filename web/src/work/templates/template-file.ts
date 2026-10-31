// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Issue templates as the team settings file holds them (#144, D313). A
 * template fills the fields a new issue leaves empty; `repeat` makes the
 * issues made from it repeat. The server checks the whole file on save.
 */

/** One template in `team.yaml`. */
export interface Template {
  readonly name: string;
  readonly title?: string;
  readonly state?: string;
  /** 0 none, 1 urgent … 4 low. */
  readonly priority?: number;
  /** Points, or a t-shirt size such as `M`. */
  readonly estimate?: number | string;
  readonly assignee?: string;
  readonly labels?: readonly string[];
  readonly project?: string;
  readonly milestone?: string;
  /** `1w`, `2w`, `1m`, `3d`. */
  readonly repeat?: string;
  readonly body?: string;
}

/** The repeats offered in a menu; any other valid one is kept. */
const REPEATS = ["1d", "1w", "2w", "1m", "3m"] as const;

/** The longest name a template may have. */
const MAX_NAME = 80;

/**
 * Whether `text` is a repeat the server accepts: 1 to 366 days, weeks or
 * months.
 */
export function isRepeat(text: string): boolean {
  const match = /^(\d{1,3})[dwm]$/i.exec(text);
  if (match === null) return false;
  const every = Number(match[1]);
  return every >= 1 && every <= 366;
}

/** The menu of repeats, plus `current` when it is not one of them. */
export function repeatChoices(current: string | undefined): readonly string[] {
  const known: readonly string[] = REPEATS;
  return current === undefined || current === "" || known.includes(current)
    ? known
    : [...known, current];
}

/** An estimate as typed: whole points, or a t-shirt size. */
function estimateFrom(text: string): number | string | undefined {
  const value = text.trim();
  if (value === "") return undefined;
  return /^\d+$/.test(value) ? Number(value) : value.toUpperCase();
}

/** The fields of a template form, as typed. */
export interface TemplateForm {
  readonly name: string;
  readonly title: string;
  readonly state: string;
  readonly priority: string;
  readonly estimate: string;
  readonly assignee: string;
  readonly labels: string;
  readonly project: string;
  readonly repeat: string;
  readonly body: string;
}

/** A form filled in from a template, for editing. */
export function formOf(template?: Template): TemplateForm {
  const given: Template = template ?? { name: "" };
  const text = (value: string | number | undefined) =>
    value === undefined ? "" : String(value);
  return {
    name: given.name,
    title: text(given.title),
    state: text(given.state),
    priority: text(given.priority),
    estimate: text(given.estimate),
    assignee: given.assignee === undefined ? "" : `@${given.assignee}`,
    labels: (given.labels ?? []).join(", "),
    project: text(given.project),
    repeat: text(given.repeat),
    body: text(given.body),
  };
}

/**
 * The template a form describes. Fields the form does not show (the
 * milestone, say) come over from `before`; empty fields are left out.
 */
export function templateOf(form: TemplateForm, before?: Template): Template {
  const labels = form.labels
    .split(",")
    .map((label) => label.trim())
    .filter((label) => label !== "");
  const entries: [string, unknown][] = [
    ["name", form.name.trim()],
    ["title", form.title.trim()],
    ["state", form.state],
    ["priority", form.priority === "" ? undefined : Number(form.priority)],
    ["estimate", estimateFrom(form.estimate)],
    ["assignee", form.assignee.trim().replace(/^@/, "").toLowerCase()],
    ["labels", labels],
    ["project", form.project],
    [
      "milestone",
      form.project === (before?.project ?? "") ? before?.milestone : undefined,
    ],
    ["repeat", form.repeat.trim().toLowerCase()],
    ["body", form.body],
  ];
  return Object.fromEntries(
    entries.filter(
      ([, value]) =>
        value !== undefined &&
        value !== "" &&
        !(Array.isArray(value) && value.length === 0),
    ),
  ) as unknown as Template;
}

/** What is wrong with a form, as a message id, or undefined when it is fine. */
export function problemOf(
  form: TemplateForm,
  otherNames: readonly string[],
): string | undefined {
  const name = form.name.trim();
  if (name === "") return "tpl-error-name";
  if (name.length > MAX_NAME) return "tpl-error-name-long";
  if (otherNames.some((other) => other.toLowerCase() === name.toLowerCase())) {
    return "tpl-error-name-taken";
  }
  if (form.title.includes("\n")) return "tpl-error-title";
  const repeat = form.repeat.trim();
  if (repeat !== "" && !isRepeat(repeat)) return "tpl-error-repeat";
  const estimate = form.estimate.trim();
  if (estimate !== "" && !/^(\d{1,4}|xs|s|m|l|xl)$/i.test(estimate)) {
    return "tpl-error-estimate";
  }
  const assignee = form.assignee.trim().replace(/^@/, "").toLowerCase();
  if (assignee !== "" && !/^[a-z0-9][a-z0-9._-]{0,63}$/.test(assignee)) {
    return "tpl-error-assignee";
  }
  return undefined;
}

/** A name for a copy of `name` that no other template has. */
export function copyName(name: string, taken: readonly string[]): string {
  const lower = new Set(taken.map((each) => each.toLowerCase()));
  let candidate = `${name} copy`;
  let n = 2;
  while (lower.has(candidate.toLowerCase())) {
    candidate = `${name} copy ${String(n)}`;
    n += 1;
  }
  return candidate.slice(0, MAX_NAME);
}

/** Moves the template at `index` one place up or down. */
export function moved<T>(list: readonly T[], index: number, step: -1 | 1): T[] {
  const out = [...list];
  const other = index + step;
  const a = out[index];
  const b = out[other];
  if (a === undefined || b === undefined) return out;
  out[index] = b;
  out[other] = a;
  return out;
}
