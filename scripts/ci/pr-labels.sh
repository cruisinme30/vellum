#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Sets the type/*, size/* and release labels on a pull request, then fails
# unless it carries at least one type/* and one area/* label.
# Environment: PR (number), TITLE, BASE, HEAD_REF; gh must be able to write.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/ci/commit-rules.sh
source "${script_dir}/commit-rules.sh"

current="$(gh pr view "${PR}" --json labels --jq '.labels[].name')"

# replace_group <prefix> <label>: keep only <label> within the group.
replace_group() {
  local prefix="$1" wanted="$2" label
  while read -r label; do
    if [[ -n "${label}" && "${label}" == "${prefix}"* && "${label}" != "${wanted}" ]]; then
      gh pr edit "${PR}" --remove-label "${label}"
    fi
  done <<<"${current}"
  if ! grep -qxF "${wanted}" <<<"${current}"; then
    gh pr edit "${PR}" --add-label "${wanted}"
  fi
}

# Type from the Conventional Commits title.
if check_subject "${TITLE}"; then
  type="${TITLE%%[(:!]*}"
  replace_group "type/" "type/${type}"
fi

# Size from changed lines, excluding generated files.
lines="$(gh pr view "${PR}" --json files --jq \
  '[.files[] | select(.path | test("(^|/)(Cargo\\.lock|pnpm-lock\\.yaml)$|^LICENSES?/") | not)
   | .additions + .deletions] | add // 0')"
if ((lines < 50)); then size=xs
elif ((lines < 200)); then size=s
elif ((lines < 500)); then size=m
elif ((lines < 1000)); then size=l
else size=xl
fi
replace_group "size/" "size/${size}"

if [[ "${BASE}" == "main" && "${HEAD_REF}" == "develop" ]]; then
  gh pr edit "${PR}" --add-label release
fi

labels="$(gh pr view "${PR}" --json labels --jq '.labels[].name')"
missing=()
grep -q '^type/' <<<"${labels}" || missing+=("type/*")
grep -q '^area/' <<<"${labels}" || missing+=("area/*")
if ((${#missing[@]} > 0)); then
  echo "::error::Missing ${missing[*]} label(s). Fix the title, or add an area/* label by hand."
  exit 1
fi
echo "Labels: $(tr '\n' ' ' <<<"${labels}")"
