#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Keeps decision records short and alike (docs/adr/0000-template.md):
# - the sections Context, Decisions, Alternatives and Consequences, in
#   that order and no others, none of them empty;
# - Issue and Pull requests lines in the facts block;
# - at most 150 lines, and at most 6 bullets in a row under any heading.
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
  ((lines > max_lines)) && fail "${adr}" "${lines} lines; keep a decision record under ${max_lines}."

  headings="$(grep -E '^## ' "${adr}" | sed 's/^## //' | tr '\n' ' ' | sed 's/ $//')"
  [[ "${headings}" == "${expected}" ]] ||
    fail "${adr}" "Sections are '${headings}'; use exactly: ${expected}."

  for fact in Issue "Pull request"; do
    grep -qE "^- ${fact}s?: " "${adr}" || fail "${adr}" "The facts block needs a '- ${fact}:' line."
  done

  # Empty sections, and runs of more than max_bullets top-level bullets.
  problems="$(awk -v max="${max_bullets}" '
    /^```/ { fence = !fence }
    fence { next }
    /^## / {
      if (section != "" && !content) print "Section \"" section "\" is empty."
      section = $0; sub(/^## /, "", section); heading = section; content = 0; run = 0; next
    }
    /^###+ / { heading = $0; sub(/^#+ /, "", heading); content = 1; run = 0; next }
    /^- / { run++; if (run == max + 1) print "\"" heading "\" has more than " max " bullets." }
    /^[^ -]/ { run = 0 }
    /[^[:space:]]/ { content = 1 }
    END { if (section != "" && !content) print "Section \"" section "\" is empty." }
  ' "${adr}")"
  while IFS= read -r problem; do
    [[ -n "${problem}" ]] && fail "${adr}" "${problem}"
  done <<<"${problems}"
done

if ((failures > 0)); then
  echo "See docs/adr/0000-template.md for the shape of a decision record."
  exit 1
fi
echo "Every decision record follows the template."
