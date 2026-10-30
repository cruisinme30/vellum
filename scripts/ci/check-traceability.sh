#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Every change is traceable: its pull request links a GitHub issue that holds
# the discussion, and a feature also records its architecture decisions in
# docs/adr/. Exempt: release pull requests (develop into main), bots, and
# chore or ci pull requests.
#
# Environment: PR_BODY, TITLE, BASE, AUTHOR_TYPE, BASE_SHA, HEAD_SHA,
# GH_REPO and GH_TOKEN (to look the issues up). `cargo xtask pr-check` sets
# ISSUE_LOOKUP=off when GitHub is unreachable; the numbers are then not
# looked up.
set -euo pipefail

type="${TITLE%%[(:!]*}"
if [[ "${BASE}" == "main" || "${AUTHOR_TYPE:-}" == "Bot" \
  || "${type}" == "chore" || "${type}" == "ci" ]]; then
  echo "Exempt from traceability (release, bot, chore or ci pull request)."
  exit 0
fi

# Ignore the template's hints, which mention "Closes #" as an example.
body="$(printf '%s\n' "${PR_BODY:-}" | tr -d '\r' \
  | perl -0pe 's/<!--.*?-->//gs')"

mapfile -t issues < <(grep -oiE \
  '\b(close[sd]?|fix(e[sd])?|resolve[sd]?|refs?)[: ]+#[0-9]+' <<<"${body}" |
  grep -oE '[0-9]+$' | sort -un)

if ((${#issues[@]} == 0)); then
  echo "::error::Link the issue that holds this change's discussion: add" \
    "'Closes #N' (or 'Refs #N') to the description."
  exit 1
fi

failures=0
for number in "${issues[@]}"; do
  if [[ "${ISSUE_LOOKUP:-on}" == "off" ]]; then
    echo "Linked #${number}; not looked up here, CI checks it is an issue."
  elif ! kind="$(gh api "repos/${GH_REPO}/issues/${number}" \
    --jq 'if .pull_request then "pull request" else "issue" end' \
    2>/dev/null)"; then
    echo "::error::#${number} does not exist in ${GH_REPO}."
    failures=$((failures + 1))
  elif [[ "${kind}" != "issue" ]]; then
    echo "::error::#${number} is a pull request; link an issue."
    failures=$((failures + 1))
  else
    echo "Linked issue #${number}."
  fi
done

if [[ "${type}" == "feat" ]]; then
  # A feature adds or updates a decision record that links one of its issues,
  # or names an existing record in its description.
  recorded=false
  while IFS= read -r adr; do
    [[ -z "${adr}" || "$(basename "${adr}")" == 0000-* || ! -f "${adr}" ]] \
      && continue
    for number in "${issues[@]}"; do
      if grep -qE "(#${number}\b|/issues/${number}\b)" "${adr}"; then
        echo "Decision record ${adr} links #${number}."
        recorded=true
      fi
    done
  done < <(git diff --name-only --diff-filter=AM "${BASE_SHA}...${HEAD_SHA}" \
    -- 'docs/adr/*.md')

  if [[ "${recorded}" == false ]]; then
    while IFS= read -r number; do
      if compgen -G "docs/adr/${number}-*.md" >/dev/null; then
        echo "Description names the existing decision record ADR ${number}."
        recorded=true
      fi
    done < <(grep -oE '\bADR ?[0-9]{4}\b' <<<"${body}" | grep -oE '[0-9]{4}' \
      | grep -v '^0000$' | sort -u)
  fi

  if [[ "${recorded}" == false ]]; then
    echo "::error::A feature needs a decision record: add or update" \
      "docs/adr/NNNN-<slug>.md (from docs/adr/0000-template.md) linking" \
      "the issue, or name an existing record as 'ADR NNNN' in the" \
      "description."
    failures=$((failures + 1))
  fi
fi

if ((failures > 0)); then
  exit 1
fi
echo "The change is traceable."
