// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState } from "react";

import { useT } from "../i18n/i18n";

import { choiceValue, isChoice, isMultiple, useChoices } from "./editing";
import { OptionPicker } from "./option-picker";
import type { PropertyInfo, Value } from "./types";
import { inputText, parseInput, texts } from "./value";

interface ValueEditorProperties {
  readonly property: PropertyInfo;
  readonly value: Value | undefined;
  /** The accessible name of the editor. */
  readonly label: string;
  readonly onCommit: (value: Value) => void;
  readonly onCancel: () => void;
}

function ChoiceEditor({
  property,
  value,
  label,
  onCommit,
  onCancel,
}: ValueEditorProperties) {
  const choices = useChoices(property);
  return (
    <OptionPicker
      label={label}
      choices={choices}
      selected={texts(value)}
      multiple={isMultiple(property.type)}
      onCommit={(ids) => {
        onCommit(choiceValue(property.type, ids));
      }}
      onCancel={onCancel}
    />
  );
}

/**
 * A text field for titles, text, URLs, numbers and dates. Enter keeps the
 * change (or explains why it cannot), Escape drops it, and leaving the
 * field keeps it when it parses.
 */
function TextEditor({
  property,
  value,
  label,
  onCommit,
  onCancel,
}: ValueEditorProperties) {
  const t = useT();
  const errorId = useId();
  const input = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  const [draft, setDraft] = useState(() => inputText(value));
  const [error, setError] = useState<string>();
  useEffect(() => {
    input.current?.focus();
  }, []);

  const commit = (leaving: boolean) => {
    const parsed = parseInput(property.type, draft);
    if ("error" in parsed) {
      if (leaving) {
        done.current = true;
        onCancel();
      } else {
        setError(t(parsed.error));
      }
      return;
    }
    done.current = true;
    onCommit(parsed.value);
  };

  return (
    <span className="vl-editor">
      <input
        ref={input}
        className="vl-editor-input"
        type={property.type === "date" ? "date" : "text"}
        inputMode={property.type === "number" ? "decimal" : undefined}
        aria-label={label}
        aria-invalid={error === undefined ? undefined : true}
        aria-describedby={error === undefined ? undefined : errorId}
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
          setError(undefined);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit(false);
          } else if (event.key === "Escape") {
            event.preventDefault();
            done.current = true;
            onCancel();
          }
          if (event.key !== "Tab") {
            event.stopPropagation();
          }
        }}
        onBlur={() => {
          if (!done.current) {
            commit(true);
          }
        }}
      />
      {error !== undefined && (
        <span id={errorId} className="vl-field-error" role="alert">
          {error}
        </span>
      )}
    </span>
  );
}

/** The editor for one value, chosen by the property's type. */
export function ValueEditor(properties: ValueEditorProperties) {
  return isChoice(properties.property.type) ? (
    <ChoiceEditor {...properties} />
  ) : (
    <TextEditor {...properties} />
  );
}
