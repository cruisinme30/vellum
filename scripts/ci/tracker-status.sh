#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Keeps the 1.0 tracker current: every task issue a pull request closes
# ("Closes #N", "Fixes #N", "Resolves #N") is labelled status/in-progress
# while the pull request is open, and loses that label when it closes.
# GitHub closes the issue itself when the pull request merges into the
# default branch. Environment: PR_BODY, PR_STATE (open | closed); gh must
# be able to write issues.
set -euo pipefail

mapfile -t issues < <(
  printf '%s\n' "${PR_BODY:-}" |
    grep -oiE '\b(close[sd]?|fix(e[sd])?|resolve[sd]?) #[0-9]+' |
    grep -oE '[0-9]+' | sort -un || true
)

if ((${#issues[@]} == 0)); then
  echo "No task issues are closed by this pull request."
  exit 0
fi

for n in "${issues[@]}"; do
  labels="$(gh issue view "${n}" --json labels \
    --jq '.labels[].name' 2>/dev/null || true)"
  if ! grep -qxF task <<<"${labels}"; then
    echo "#${n} is not a task issue; skipped."
    continue
  fi
  if [[ "${PR_STATE}" == "open" ]]; then
    gh issue edit "${n}" --add-label status/in-progress >/dev/null
    echo "#${n} is in progress."
  elif grep -qxF status/in-progress <<<"${labels}"; then
    gh issue edit "${n}" --remove-label status/in-progress >/dev/null
    echo "#${n} is no longer in progress."
  fi
done
