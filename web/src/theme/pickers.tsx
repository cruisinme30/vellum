// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId } from "react";

import { useT } from "../i18n/i18n";

import { effectiveSeed, type ThemeChoice } from "./appearance";
import { useAppearance } from "./appearance-context";
import { Choices } from "./choices";
import { THEMES } from "./themes";

/** The theme radio buttons, with the custom theme once there is one (R-TH-1). */
export function ThemePicker({ name }: { readonly name: string }) {
  const t = useT();
  const { appearance, update } = useAppearance();
  const { custom } = appearance;
  const options: readonly ThemeChoice[] = custom === undefined ? THEMES : [...THEMES, "custom"];
  return (
    <Choices
      legend={t("theme-legend")}
      name={name}
      options={options}
      value={appearance.theme}
      label={(theme) =>
        theme === "custom" && custom !== undefined
          ? t("theme-custom-named", { name: custom.name })
          : t(`theme-${theme}`)
      }
      onChange={(theme) => {
        update({ theme });
      }}
    />
  );
}

/**
 * The accent colour: the user's own, or the organisation's (R-TH-1). The
 * reset button goes back to the organisation's colour, or Vellum's in demo
 * mode.
 */
export function SeedPicker() {
  const t = useT();
  const { appearance, update, organisation } = useAppearance();
  const seedId = useId();
  return (
    <div className="vl-seed">
      <label htmlFor={seedId}>{t("theme-seed")}</label>
      <input
        id={seedId}
        type="color"
        value={effectiveSeed(appearance, organisation?.seed)}
        onChange={(event) => {
          update({ seed: event.target.value });
        }}
      />
      <button
        type="button"
        className="vl-text-button"
        disabled={appearance.seed === undefined}
        onClick={() => {
          update({ seed: undefined });
        }}
      >
        {organisation === undefined ? t("theme-seed-reset") : t("theme-seed-organisation")}
      </button>
    </div>
  );
}
