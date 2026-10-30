#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Keeps decision records short and alike (docs/adr/0000-template.md):
# - the sections Context, Decisions, Alternatives and Consequences, in
#   that order and no others, none of them empty;
# - Issue and Pull requests lines in the facts block;
# - at most 150 lines, and at most 6 bullets in a row under any heading;
# - no ADR number or D number used twice, and no overlapping reservations
#   in docs/adr/reserved.toml. Threads pick their own numbers
#   with `cargo xtask adr next` (ADR 0049, D216), so this is what catches
#   two branches that picked the same ones.
set -euo pipefail

max_lines=150
max_bullets=6
expected="Context Decisions Alternatives Consequences"

failures=0
fail() {
  echo "::error file=$1::$2"
  failures=$((failures + 1))
}

for adr in docs/adr/[0-9]*.md; do
  [[ "$(basename "${adr}")" == 0000-* ]] && continue

  lines="$(wc -l <"${adr}")"
  ((lines > max_lines)) && fail "${adr}" \
    "${lines} lines; keep a decision record under ${max_lines}."

  headings="$(grep -E '^## ' "${adr}" | sed 's/^## //' | tr '\n' ' ' \
    | sed 's/ $//')"
  [[ "${headings}" == "${expected}" ]] ||
    fail "${adr}" "Sections are '${headings}'; use exactly: ${expected}."

  for fact in Issue "Pull request"; do
    grep -qE "^- ${fact}s?: " "${adr}" || fail "${adr}" \
      "The facts block needs a '- ${fact}:' line."
  done

  # Empty sections, and runs of more than max_bullets top-level bullets.
  problems="$(awk -v max="${max_bullets}" '
    /^```/ { fence = !fence }
    fence { next }
    /^## / {
      if (section != "" && !content) print "Section \"" section "\" is empty."
      section = $0; sub(/^## /, "", section); heading = section; content = 0;
        run = 0; next
    }
    /^###+ / { heading = $0; sub(/^#+ /, "", heading); content = 1; run = 0;
      next }
    /^- / { run++;
      if (run == max + 1)
        print "\"" heading "\" has more than " max " bullets." }
    /^[^ -]/ { run = 0 }
    /[^[:space:]]/ { content = 1 }
    END { if (section != "" &&
      !content) print "Section \"" section "\" is empty." }
  ' "${adr}")"
  while IFS= read -r problem; do
    [[ -n "${problem}" ]] && fail "${adr}" "${problem}"
  done <<<"${problems}"
done

# Duplicate ADR numbers (file name prefixes) and D numbers (facts lines).
declare -A adr_owner=() decision_owner=()
for adr in docs/adr/[0-9]*.md; do
  name="$(basename "${adr}")"
  [[ "${name}" == 0000-* ]] && continue
  number="${name%%-*}"
  if [[ -n "${adr_owner[${number}]:-}" ]]; then
    message="ADR ${number} is also ${adr_owner[${number}]};"
    fail "${adr}" "${message} run 'cargo xtask adr next' for a free number."
  fi
  adr_owner[${number}]="${name}"
  for decision in $(grep -m1 '^- Decisions:' "${adr}" | grep -oE '\bD[0-9]+\b' \
    || true); do
    if [[ -n "${decision_owner[${decision}]:-}" ]]; then
      message="${decision} is also in ${decision_owner[${decision}]};"
      fail "${adr}" \
        "${message} run 'cargo xtask adr next' for a free number."
    fi
    decision_owner[${decision}]="${name}"
  done
done

# Reservations (docs/adr/reserved.toml) may not overlap one another.
reserved=docs/adr/reserved.toml
declare -A reserved_owner=()
if [[ -f "${reserved}" ]]; then
  while IFS= read -r range; do
    first="${range%%-*}"
    last="${range##*-}"
    first="${first#D}"
    last="${last#D}"
    if ! [[ "${first}" =~ ^[0-9]+$ && "${last}" =~ ^[0-9]+$ ]] \
      || ((10#${first} > 10#${last})); then
      fail "${reserved}" \
        "Cannot read the range '${range}'; write \"D217\" or \"D217-D261\"."
      continue
    fi
    for ((d = 10#${first}; d <= 10#${last}; d++)); do
      if [[ -n "${reserved_owner[${d}]:-}" ]]; then
        fail "${reserved}" \
          "D${d} is in both '${reserved_owner[${d}]}' and '${range}'."
        break
      fi
      reserved_owner[${d}]="${range}"
    done
  done < <(sed -nE 's/^decisions *= *"([^"]+)".*/\1/p' "${reserved}")
fi

if ((failures > 0)); then
  echo "See docs/adr/0000-template.md for the shape of a decision record."
  exit 1
fi
echo "Every decision record follows the template."
