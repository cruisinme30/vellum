// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId } from "react";

import { useT } from "../i18n/i18n";
import { useAppearance } from "../theme/appearance-context";
import { DEFAULT_SEED, THEMES } from "../theme/themes";

/** Theme, seed colour and density (R-TH-1, R-TH-5). */
export function ThemeSettings() {
  const t = useT();
  const { appearance, update } = useAppearance();
  const seedId = useId();
  return (
    <div className="vl-theme-settings">
      <fieldset>
        <legend>{t("theme-legend")}</legend>
        {THEMES.map((theme) => (
          <label key={theme} className="vl-radio">
            <input
              type="radio"
              name="vl-theme"
              value={theme}
              checked={appearance.theme === theme}
              onChange={() => {
                update({ theme });
              }}
            />
            {t(`theme-${theme}`)}
          </label>
        ))}
      </fieldset>
      <fieldset>
        <legend>{t("density-legend")}</legend>
        {(["comfortable", "compact"] as const).map((density) => (
          <label key={density} className="vl-radio">
            <input
              type="radio"
              name="vl-density"
              value={density}
              checked={appearance.density === density}
              onChange={() => {
                update({ density });
              }}
            />
            {t(`density-${density}`)}
          </label>
        ))}
      </fieldset>
      <div className="vl-seed">
        <label htmlFor={seedId}>{t("theme-seed")}</label>
        <input
          id={seedId}
          type="color"
          value={appearance.seed}
          onChange={(event) => {
            update({ seed: event.target.value });
          }}
        />
        <button
          type="button"
          className="vl-text-button"
          disabled={appearance.seed === DEFAULT_SEED}
          onClick={() => {
            update({ seed: DEFAULT_SEED });
          }}
        >
          {t("theme-seed-reset")}
        </button>
      </div>
    </div>
  );
}
