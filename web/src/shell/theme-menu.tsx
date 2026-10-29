// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link } from "react-router";

import { useT } from "../i18n/i18n";
import { DENSITIES } from "../theme/appearance";
import { useAppearance } from "../theme/appearance-context";
import { Choices } from "../theme/choices";
import { SeedPicker, ThemePicker } from "../theme/pickers";

/** The quick theme menu: theme, accent colour and density (R-TH-1, R-TH-5). */
export function ThemeSettings() {
  const t = useT();
  const { appearance, update } = useAppearance();
  return (
    <div className="vl-theme-settings">
      <ThemePicker name="vl-theme" />
      <Choices
        legend={t("density-legend")}
        name="vl-density"
        options={DENSITIES}
        value={appearance.density}
        label={(density) => t(`density-${density}`)}
        onChange={(density) => {
          update({ density });
        }}
      />
      <SeedPicker />
      <Link className="vl-theme-more" to="/settings/appearance">
        {t("appearance-more")}
      </Link>
    </div>
  );
}
