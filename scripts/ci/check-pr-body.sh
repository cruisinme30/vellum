#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Checks that a pull request description explains itself
# (.github/pull_request_template.md). Reads the body from $PR_BODY.
# - Every required section is present and filled in.
# - A decision (D12) or record (ADR 0012) is never named bare: its line
#   also carries a link and a few words saying what it is.
set -euo pipefail

required=(Why "What changed" "How to verify" "Security and operations")

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

# A line naming D<n> or ADR <n> needs a link and at least four other words.
while IFS= read -r line; do
  [[ -z "${line}" ]] && continue
  words="$(perl -pe 's/\]\([^)]*\)/]/g; s/\bADR ?\d{4}\b//g; s/\bD\d{1,3}\b//g; s/[^A-Za-z ]/ /g' <<<"${line}" | wc -w)"
  if [[ "${line}" != *"]("* || "${words}" -lt 4 ]]; then
    echo "::error::Explain this reference in plain words and link it: ${line}"
    failures=$((failures + 1))
  fi
done < <(grep -E '\b(D[0-9]{1,3}|ADR ?[0-9]{4})\b' <<<"${body}" | grep -vE '^\s*```' || true)

if ((failures > 0)); then
  echo "See .github/pull_request_template.md: a reader should follow the description without knowing project IDs."
  exit 1
fi
echo "The description explains itself."
