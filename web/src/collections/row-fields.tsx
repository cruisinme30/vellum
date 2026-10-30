// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";

import { useT } from "../i18n/i18n";

import { Chips } from "./cell-content";
import {
  choiceLabels,
  choiceValue,
  isChoice,
  isMultiple,
  useChoices,
} from "./editing";
import { OptionPicker } from "./option-picker";
import type { CellValue, PropertyInfo, Value } from "./types";
import {
  bool,
  cellValue,
  formatNumber,
  inputText,
  parseInput,
  plain,
  sameValue,
  texts,
} from "./value";

interface FieldProperties {
  readonly property: PropertyInfo;
  readonly value: Value | undefined;
  /** Saves a value; resolves to the error message when it was refused. */
  readonly onSave: (value: Value) => Promise<string | undefined>;
}

function FieldError({
  id,
  error,
}: {
  readonly id: string;
  readonly error: string | undefined;
}) {
  return error === undefined ? undefined : (
    <span id={id} className="vl-field-error" role="alert">
      {error}
    </span>
  );
}

/** Saves a value and keeps the error, if any, to show under the field. */
function useSave(onSave: FieldProperties["onSave"]) {
  const [error, setError] = useState<string>();
  const save = (value: Value) => {
    void onSave(value).then(setError);
  };
  return { error, setError, save };
}

function InputField({ property, value, onSave }: FieldProperties) {
  const t = useT();
  const errorId = useId();
  const [draft, setDraft] = useState(() => inputText(value));
  const { error, setError, save } = useSave(onSave);
  const commit = () => {
    const parsed = parseInput(property.type, draft);
    if ("error" in parsed) {
      setError(t(parsed.error));
    } else if (!sameValue(parsed.value, value)) {
      save(parsed.value);
    }
  };
  return (
    <label className="vl-field vl-row-field">
      {property.name}
      <input
        type={property.type === "date" ? "date" : "text"}
        inputMode={property.type === "number" ? "decimal" : undefined}
        data-title={property.type === "title" || undefined}
        value={draft}
        aria-invalid={error === undefined ? undefined : true}
        aria-describedby={error === undefined ? undefined : errorId}
        onChange={(event) => {
          setDraft(event.target.value);
        }}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          }
        }}
      />
      <FieldError id={errorId} error={error} />
    </label>
  );
}

function ChoiceField({ property, value, onSave }: FieldProperties) {
  const t = useT();
  const errorId = useId();
  const labelId = useId();
  const choices = useChoices(property);
  const [open, setOpen] = useState(false);
  const { error, save } = useSave(onSave);
  const labels = choiceLabels(choices, value);
  return (
    <div className="vl-field vl-row-field vl-menu-anchor">
      <span id={labelId}>{property.name}</span>
      <button
        type="button"
        className="vl-choice-button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-labelledby={`${labelId} ${labelId}-value`}
        aria-describedby={error === undefined ? undefined : errorId}
        onClick={() => {
          setOpen(!open);
        }}
      >
        <span id={`${labelId}-value`}>
          {labels.length === 0 ? (
            <span className="vl-muted">{t("collection-none")}</span>
          ) : (
            <Chips names={labels} options={property.options} />
          )}
        </span>
      </button>
      {open && (
        <OptionPicker
          label={property.name}
          choices={choices}
          selected={texts(value)}
          multiple={isMultiple(property.type)}
          onCommit={(ids) => {
            setOpen(false);
            const next = choiceValue(property.type, ids);
            if (!sameValue(next, value)) {
              save(next);
            }
          }}
          onCancel={() => {
            setOpen(false);
          }}
        />
      )}
      <FieldError id={errorId} error={error} />
    </div>
  );
}

function CheckField({ property, value, onSave }: FieldProperties) {
  const errorId = useId();
  const { error, save } = useSave(onSave);
  const checked = value?.type === "bool" && value.value;
  return (
    <div className="vl-row-field">
      <label className="vl-radio">
        <input
          type="checkbox"
          checked={checked}
          aria-describedby={error === undefined ? undefined : errorId}
          onChange={() => {
            save(bool(!checked));
          }}
        />
        {property.name}
      </label>
      <FieldError id={errorId} error={error} />
    </div>
  );
}

/** A computed value as text, as the server formats it. */
function computedText(
  property: PropertyInfo,
  value: Value | undefined,
): string {
  if (value?.type === "number" && property.format === "percent") {
    return `${formatNumber(Math.round(value.value * 100))}%`;
  }
  return plain(value);
}

/** One property of a row in the side panel, editable unless computed. */
export function RowField({
  property,
  cell,
  onSave,
}: {
  readonly property: PropertyInfo;
  readonly cell: CellValue | undefined;
  readonly onSave: FieldProperties["onSave"];
}) {
  const t = useT();
  const labelId = useId();
  const value = cellValue(cell);
  if (property.computed) {
    const problem =
      cell !== undefined && "error" in cell ? cell.error.message : undefined;
    return (
      <div className="vl-field vl-row-field">
        <span id={labelId}>{property.name}</span>
        <output aria-labelledby={labelId} className="vl-row-computed">
          {problem === undefined
            ? computedText(property, value)
            : `⚠ ${problem}`}
        </output>
        <span className="vl-visually-hidden">{t("collection-computed")}</span>
      </div>
    );
  }
  if (property.type === "checkbox") {
    return <CheckField property={property} value={value} onSave={onSave} />;
  }
  if (isChoice(property.type)) {
    return <ChoiceField property={property} value={value} onSave={onSave} />;
  }
  return (
    <InputField
      key={JSON.stringify(value)}
      property={property}
      value={value}
      onSave={onSave}
    />
  );
}
