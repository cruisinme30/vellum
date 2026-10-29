// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Conditional skips say why in their message: a browser or setup lacks the feature.
/* eslint-disable sonarjs/no-skipped-tests */

import { expect, test } from "./fixtures";
import { auditTarget, describe } from "./harness";
import { triage } from "./known";
import { MODES } from "./modes";
import { TARGETS } from "./targets";

/**
 * Every target in every display mode, in the project's browser and
 * viewport (ADR 0031, D123–D124). A failure lists each problem with the
 * element it is about; the full-page screenshot is attached.
 */
for (const target of TARGETS) {
  test.describe(target.name, () => {
    for (const mode of MODES) {
      if (mode.appOnly === true && !target.app) continue;
      test(mode.name, async ({ page, browserName }, testInfo) => {
        test.skip(
          mode.chromiumOnly === true && browserName !== "chromium",
          "Chromium-only media feature",
        );
        const problems = await auditTarget(page, target, mode, testInfo);
        const { fresh, known } = triage(problems, `${target.name} › ${mode.name}`);
        for (const description of known)
          testInfo.annotations.push({ type: "known issue", description });
        expect(describe(fresh)).toEqual([]);
      });
    }
  });
}
