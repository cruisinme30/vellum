// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** A labelled group of radio buttons. */
export function Choices<T extends string | number>({
  legend,
  name,
  options,
  value,
  label,
  onChange,
}: {
  readonly legend: string;
  readonly name: string;
  readonly options: readonly T[];
  readonly value: T;
  readonly label: (option: T) => string;
  readonly onChange: (option: T) => void;
}) {
  return (
    <fieldset className="vl-choices">
      <legend>{legend}</legend>
      {options.map((option) => (
        <label key={String(option)} className="vl-radio">
          <input
            type="radio"
            name={name}
            value={String(option)}
            checked={value === option}
            onChange={() => {
              onChange(option);
            }}
          />
          {label(option)}
        </label>
      ))}
    </fieldset>
  );
}
