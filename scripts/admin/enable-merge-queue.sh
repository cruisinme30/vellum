#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Turns on the merge queue for develop: pull requests are queued and merged
# one group at a time, each tested on top of the ones ahead of it, instead
# of racing each other. Also allows auto-merge, which is how a pull request
# joins the queue (gh pr merge --auto --squash).
#
# Needs a repository admin: run it once with `gh` signed in as the owner.
# Run it only after the merge_group triggers are on develop, or the queue
# waits forever for checks that never start.
set -euo pipefail

repo="${1:-cruisinme30/vellum}"

gh api --method PATCH "repos/${repo}" -F allow_auto_merge=true >/dev/null
echo "Auto-merge allowed on ${repo}."

ruleset_id="$(gh api "repos/${repo}/rulesets" --jq '
  map(select(.target == "branch")) | .[].id' | while read -r id; do
    if gh api "repos/${repo}/rulesets/${id}" --jq '.conditions.ref_name.include[]' |
      grep -qx 'refs/heads/develop'; then
      echo "${id}"
    fi
  done | head -n1)"

if [[ -z "${ruleset_id}" ]]; then
  echo "No ruleset targets develop; create it first (see CONTRIBUTING.md)." >&2
  exit 1
fi

queue_rule='{
  "type": "merge_queue",
  "parameters": {
    "merge_method": "SQUASH",
    "grouping_strategy": "ALLGREEN",
    "max_entries_to_build": 5,
    "min_entries_to_merge": 1,
    "max_entries_to_merge": 5,
    "min_entries_to_merge_wait_minutes": 1,
    "check_response_timeout_minutes": 60
  }
}'

gh api "repos/${repo}/rulesets/${ruleset_id}" |
  jq --argjson queue "${queue_rule}" '{
    name, target, enforcement, conditions, bypass_actors,
    rules: ([.rules[] | select(.type != "merge_queue")] + [$queue])
  }' |
  gh api --method PUT "repos/${repo}/rulesets/${ruleset_id}" --input - >/dev/null

echo "Merge queue enabled on develop (ruleset ${ruleset_id})."
