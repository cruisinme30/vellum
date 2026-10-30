// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Triage rules and SLA rules (R-WK-9): ordered lists, first match wins,
 * each with a condition built with the same filter builder as views.
 */

import { useState, type ReactNode } from "react";

import { useT } from "../../i18n/i18n";
import { PRIORITIES } from "../chips";
import { parse, serialize } from "../filter";
import { FilterBuilder } from "../filter-builder";

import { Check, Choice, NumberField, Text, type Edit } from "./sections";
import {
  without,
  type FilterJson,
  type SlaRule,
  type TeamFile,
  type TriageRule,
} from "./team-file";

const EVERY: FilterJson = { and: [] };

/** A triage rule with its actions; the file format names them `then`. */
function withActions(
  rule: Omit<TriageRule, "then">,
  actions: TriageRule["then"],
): TriageRule {
  // The field's name in team.yaml.
  // eslint-disable-next-line unicorn/no-thenable
  return { ...rule, then: actions };
}

function moved<T>(list: readonly T[], at: number, step: -1 | 1): T[] {
  const next = [...list];
  const a = next[at];
  const b = next[at + step];
  if (a === undefined || b === undefined) return next;
  next[at] = b;
  next[at + step] = a;
  return next;
}

function Condition({
  when,
  statuses,
  onChange,
}: {
  readonly when: FilterJson;
  readonly statuses: readonly string[];
  readonly onChange: (when: FilterJson) => void;
}) {
  const t = useT();
  const text = JSON.stringify(when);
  const group = parse(text);
  if (group === undefined)
    return <p className="vl-muted">{t("work-filter-unsupported")}</p>;
  return (
    <FilterBuilder
      key={text}
      value={group}
      statuses={statuses}
      onApply={(next) => {
        const json = serialize(next);
        onChange(json === undefined ? EVERY : (JSON.parse(json) as FilterJson));
      }}
    />
  );
}

function RuleFrame({
  name,
  index,
  count,
  onRename,
  onMove,
  onRemove,
  children,
}: {
  readonly name: string;
  readonly index: number;
  readonly count: number;
  readonly onRename: (name: string) => void;
  readonly onMove: (step: -1 | 1) => void;
  readonly onRemove: () => void;
  readonly children: ReactNode;
}) {
  const t = useT();
  return (
    <li className="vl-settings-rule">
      <div className="vl-settings-rule-head">
        <Text
          label={t("team-settings-rule-name", { n: index + 1 })}
          value={name}
          onChange={onRename}
        />
        <button
          type="button"
          className="vl-text-button"
          disabled={index === 0}
          aria-label={t("team-settings-up", { name })}
          onClick={() => {
            onMove(-1);
          }}
        >
          ↑
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={index === count - 1}
          aria-label={t("team-settings-down", { name })}
          onClick={() => {
            onMove(1);
          }}
        >
          ↓
        </button>
        <button type="button" className="vl-text-button" onClick={onRemove}>
          {t("team-settings-remove")}
        </button>
      </div>
      {children}
    </li>
  );
}

/** Triage on or off, and the triage rules. */
export function TriageSection({
  file,
  edit,
}: {
  readonly file: TeamFile;
  readonly edit: Edit;
}) {
  const t = useT();
  const rules = file.triage_rules ?? [];
  const statuses = file.workflow.statuses.map((status) => status.key);
  // Bumped when rules move, so rows remount with their new contents;
  // renaming keeps the row (and the focus).
  const [version, setVersion] = useState(0);
  const setRules = (
    change: (rules: readonly TriageRule[]) => readonly TriageRule[],
    reorder = false,
  ) => {
    if (reorder) setVersion((v) => v + 1);
    edit((f) => ({ ...f, triage_rules: change(f.triage_rules ?? []) }));
  };
  const setRule = (at: number, rule: TriageRule) => {
    setRules((list) => list.map((each, index) => (index === at ? rule : each)));
  };
  const hasTriageStatus = file.workflow.statuses.some(
    (status) => status.category === "triage",
  );
  return (
    <section className="vl-settings-section" aria-labelledby="settings-triage">
      <h2 id="settings-triage">{t("team-settings-triage")}</h2>
      <Check
        label={t("team-settings-triage-on")}
        checked={file.triage ?? true}
        onChange={(triage) => {
          edit((f) => ({ ...f, triage }));
        }}
      />
      {!hasTriageStatus && (
        <p className="vl-muted">{t("team-settings-triage-needs-status")}</p>
      )}
      <p className="vl-muted">{t("team-settings-rules-hint")}</p>
      <ol className="vl-settings-rules">
        {rules.map((rule, index) => (
          <RuleFrame
            // Rules have no IDs; the order is the identity, and a move
            // remounts.
            key={`${String(version)}:${String(index)}`}
            name={rule.name}
            index={index}
            count={rules.length}
            onRename={(name) => {
              setRule(index, { ...rule, name });
            }}
            onMove={(step) => {
              setRules((list) => moved(list, index, step), true);
            }}
            onRemove={() => {
              setRules(
                (list) => list.filter((_, index_) => index_ !== index),
                true,
              );
            }}
          >
            <Condition
              when={rule.when}
              statuses={statuses}
              onChange={(when) => {
                setRule(index, { ...rule, when });
              }}
            />
            <div className="vl-settings-grid">
              <Choice
                label={t("team-settings-rule-priority")}
                value={String(rule.then.priority ?? "")}
                options={[
                  ["", t("team-settings-rule-keep")],
                  ...PRIORITIES.map((name, n): [string, string] => [
                    String(n),
                    t(`work-priority-${name}`),
                  ]),
                ]}
                onChange={(value) => {
                  const actions = without(rule.then, "priority");
                  setRule(
                    index,
                    withActions(
                      rule,
                      value === ""
                        ? actions
                        : { ...actions, priority: Number(value) },
                    ),
                  );
                }}
              />
              <Text
                label={t("team-settings-rule-assignee")}
                value={rule.then.assignee ?? ""}
                onChange={(value) => {
                  const actions = without(rule.then, "assignee");
                  const handle = value.trim().replace(/^@/, "");
                  setRule(
                    index,
                    withActions(
                      rule,
                      handle === ""
                        ? actions
                        : { ...actions, assignee: handle },
                    ),
                  );
                }}
              />
            </div>
          </RuleFrame>
        ))}
      </ol>
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setRules((list) => [
            ...list,
            withActions(
              {
                name: t("team-settings-rule-new", { n: list.length + 1 }),
                when: EVERY,
              },
              {},
            ),
          ]);
        }}
      >
        {t("team-settings-add-rule")}
      </button>
    </section>
  );
}

/** SLA rules: the first matching rule sets the clock. */
export function SlaSection({
  file,
  edit,
}: {
  readonly file: TeamFile;
  readonly edit: Edit;
}) {
  const t = useT();
  const rules = file.sla?.rules ?? [];
  const statuses = file.workflow.statuses.map((status) => status.key);
  const [version, setVersion] = useState(0);
  const setRules = (
    change: (rules: readonly SlaRule[]) => readonly SlaRule[],
    reorder = false,
  ) => {
    if (reorder) setVersion((v) => v + 1);
    edit((f) => ({
      ...f,
      sla: { ...f.sla, rules: change(f.sla?.rules ?? []) },
    }));
  };
  const setRule = (at: number, rule: SlaRule) => {
    setRules((list) => list.map((each, index) => (index === at ? rule : each)));
  };
  return (
    <section className="vl-settings-section" aria-labelledby="settings-sla">
      <h2 id="settings-sla">{t("team-settings-sla")}</h2>
      <p className="vl-muted">{t("team-settings-sla-hint")}</p>
      <ol className="vl-settings-rules">
        {rules.map((rule, index) => (
          <RuleFrame
            key={`${String(version)}:${String(index)}`}
            name={rule.name}
            index={index}
            count={rules.length}
            onRename={(name) => {
              setRule(index, { ...rule, name });
            }}
            onMove={(step) => {
              setRules((list) => moved(list, index, step), true);
            }}
            onRemove={() => {
              setRules(
                (list) => list.filter((_, index_) => index_ !== index),
                true,
              );
            }}
          >
            <Condition
              when={rule.when}
              statuses={statuses}
              onChange={(when) => {
                setRule(index, { ...rule, when });
              }}
            />
            <NumberField
              label={t("team-settings-sla-hours")}
              value={rule.within_hours}
              min={1}
              max={8760}
              onChange={(within_hours) => {
                setRule(index, { ...rule, within_hours });
              }}
            />
          </RuleFrame>
        ))}
      </ol>
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setRules((list) => [
            ...list,
            {
              name: t("team-settings-rule-new", { n: list.length + 1 }),
              when: { field: "priority", op: "is", values: ["normal"] },
              within_hours: 72,
            },
          ]);
        }}
      >
        {t("team-settings-add-sla")}
      </button>
    </section>
  );
}
