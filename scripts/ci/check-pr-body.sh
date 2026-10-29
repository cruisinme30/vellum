#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Checks that a pull request description fills in every required section
# of .github/pull_request_template.md. Reads the body from $PR_BODY.
set -euo pipefail

required=(Summary Requirements Changes Tests Observability Security)

# Drop HTML comments (the template's hints), then normalise line endings.
body="$(printf '%s\n' "${PR_BODY:-}" | tr -d '\r' | perl -0pe 's/<!--.*?-->//gs')"

failures=0
for section in "${required[@]}"; do
  content="$(awk -v name="${section}" '
    /^## / { inside = ($0 == "## " name); next }
    inside { print }
  ' <<<"${body}" | tr -d '[:space:]')"
  if ! grep -qx "## ${section}" <<<"${body}"; then
    echo "::error::The description has no '## ${section}' section; use the pull request template."
    failures=$((failures + 1))
  elif [[ -z "${content}" ]]; then
    echo "::error::The '## ${section}' section is empty; fill it in or write None with a reason."
    failures=$((failures + 1))
  fi
done

if ((failures > 0)); then
  exit 1
fi
echo "Every required section of the description is filled in."
