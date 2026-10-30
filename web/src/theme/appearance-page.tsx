// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useT } from "../i18n/i18n";

import { DENSITIES, FONTS, MOTIONS, TEXT_SIZES } from "./appearance";
import { useAppearance } from "./appearance-context";
import { Choices } from "./choices";
import { SeedPicker, ThemePicker } from "./pickers";
import { ThemeEditor } from "./theme-editor";

function ThemeSection() {
  const t = useT();
  return (
    <section aria-labelledby="vl-appearance-theme" className="vl-appearance-section">
      <h2 id="vl-appearance-theme">{t("theme-title")}</h2>
      <ThemePicker name="vl-appearance-theme" />
      <SeedPicker />
    </section>
  );
}

function ReadingSection() {
  const t = useT();
  const { appearance, update } = useAppearance();
  return (
    <section aria-labelledby="vl-appearance-reading" className="vl-appearance-section">
      <h2 id="vl-appearance-reading">{t("reading-title")}</h2>
      <div className="vl-appearance-grid">
        <Choices
          legend={t("text-size-legend")}
          name="vl-text-size"
          options={TEXT_SIZES}
          value={appearance.textSize}
          label={(size) => t("text-size-option", { size })}
          onChange={(textSize) => {
            update({ textSize });
          }}
        />
        <Choices
          legend={t("font-legend")}
          name="vl-font"
          options={FONTS}
          value={appearance.font}
          label={(font) => t(`font-${font}`)}
          onChange={(font) => {
            update({ font });
          }}
        />
        <Choices
          legend={t("density-legend")}
          name="vl-density-page"
          options={DENSITIES}
          value={appearance.density}
          label={(density) => t(`density-${density}`)}
          onChange={(density) => {
            update({ density });
          }}
        />
        <Choices
          legend={t("motion-legend")}
          name="vl-motion"
          options={MOTIONS}
          value={appearance.motion}
          label={(motion) => t(`motion-${motion}`)}
          onChange={(motion) => {
            update({ motion });
          }}
        />
      </div>
      <p className="vl-appearance-sample">{t("reading-sample")}</p>
    </section>
  );
}

/** Settings → Appearance: theme, accent, reading settings and the theme editor. */
export function AppearancePage() {
  const t = useT();
  return (
    <div className="vl-view vl-appearance">
      <h1>{t("appearance-title")}</h1>
      <p className="vl-muted">{t("appearance-intro")}</p>
      <ThemeSection />
      <ReadingSection />
      <ThemeEditor />
    </div>
  );
}
