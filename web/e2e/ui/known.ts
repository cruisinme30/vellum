// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Problem } from "./harness";

/**
 * Problems the suite found that have an open issue but no fix yet. They
 * are reported as annotations instead of failures, so the suite can gate
 * every pull request without waiting for every fix. Each entry names its
 * issue; the pull request that fixes the issue deletes the entry.
 */
interface KnownIssue {
  issue: number;
  rule: string;
  /** Matched against the test's title path, e.g. "app shell › stress". */
  test: RegExp;
}

// Empty while no UI bug is open; add an entry with its issue when one is filed.
const KNOWN: readonly KnownIssue[] = [];

/** Splits problems into new ones and ones already tracked by an issue. */
export function triage(
  problems: Problem[],
  title: string,
): { fresh: Problem[]; known: string[] } {
  const fresh: Problem[] = [];
  const known: string[] = [];
  for (const problem of problems) {
    // eslint-disable-next-line sonarjs/no-empty-collection -- empty until a bug is filed
    const entry = KNOWN.find(
      (candidate) =>
        candidate.rule === problem.rule && candidate.test.test(title),
    );
    if (entry === undefined) fresh.push(problem);
    else
      known.push(
        `#${String(entry.issue)}: ${problem.rule} @ ${problem.target}`,
      );
  }
  return { fresh, known };
}
