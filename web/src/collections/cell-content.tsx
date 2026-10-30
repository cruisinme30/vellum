// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Fragment } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { usePeople } from "./editing";
import type { CellView, PropertyInfo, SelectOption } from "./types";
import { texts } from "./value";

/** A checkbox value: an icon, and its state in words for screen readers. */
function CheckMark({ checked }: { readonly checked: boolean }) {
  const t = useT();
  return (
    <span className="vl-check" data-checked={checked}>
      <Icon
        name={checked ? "check_box" : "check_box_outline_blank"}
        size={20}
      />
      <span className="vl-visually-hidden">
        {t(checked ? "collection-yes" : "collection-no")}
      </span>
    </span>
  );
}

/** Select options as coloured chips. */
export function Chips({
  names,
  options = [],
}: {
  readonly names: readonly string[];
  readonly options?: readonly SelectOption[] | undefined;
}) {
  return (
    <span className="vl-chips">
      {names.map((name, index) => (
        <Fragment key={name}>
          {/* A space between chips keeps their names apart for screen
              readers. */}
          {index > 0 && " "}
          <span
            className="vl-chip"
            data-color={options.find((o) => o.name === name)?.color}
          >
            {name}
          </span>
        </Fragment>
      ))}
    </span>
  );
}

/** How a cell of a view result reads: chips, a check mark, names or text. */
export function CellContent({
  property,
  cell,
}: {
  readonly property: PropertyInfo;
  readonly cell: CellView | undefined;
}) {
  const people = usePeople();
  if (cell?.error) {
    return <span className="vl-cell-problem">{cell.display}</span>;
  }
  const value = cell?.value;
  switch (property.type) {
    case "checkbox": {
      return <CheckMark checked={value?.type === "bool" && value.value} />;
    }
    case "select":
    case "multi": {
      return <Chips names={texts(value)} options={property.options} />;
    }
    case "person": {
      return (
        <>
          {texts(value)
            .map((handle) => people.get(handle) ?? handle)
            .join(", ")}
        </>
      );
    }
    default: {
      return <>{cell?.display ?? ""}</>;
    }
  }
}
