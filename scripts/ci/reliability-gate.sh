#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Release gate (ADR 0022, D254): a release pull request fails while the
# latest decisive run of a scheduled reliability check on develop failed:
# the k6 load test with its budgets and 10 % regression gate, the ZAP scan,
# the PDF budget in a release build, each fuzz target, and the weekly chaos
# test. "Decisive" skips the nights a job did not run (develop unchanged,
# or chaos on a weekday). A manual run of nightly.yml on develop counts, so
# a fixed failure clears the gate without waiting for the next night. A
# check that has never run is a warning, not a failure, so the first
# releases are not blocked.
#
# Reads only GitHub's API; it never re-runs the checks, so it adds seconds
# to the full suite, not hours.
#
#   GH_REPO=owner/repo GH_TOKEN=... scripts/ci/reliability-gate.sh
#   scripts/ci/reliability-gate.sh --jobs FILE   # judge a prepared list
#
# FILE (and the list built from the API) is a JSON array of jobs, newest
# run first: [{"name", "conclusion", "html_url", "completed_at"}].
set -euo pipefail

CHECKS=("Load test (k6)" "Virus scanner under hostile uploads (ClamAV)" "Security scan (ZAP baseline)"
  "PDF budget (release build)" "Chaos under load")
FUZZ_PREFIX="Fuzz "
STALE_DAYS="${STALE_DAYS:-8}"

jobs_from_api() {
  local runs ids
  runs="repos/${GH_REPO:?}/actions/workflows/nightly.yml/runs"
  ids="$(gh api "${runs}?branch=develop&status=completed&per_page=30" \
    --jq '.workflow_runs[] | select(.event == "schedule" or .event == "workflow_dispatch") | .id')"
  local id
  for id in ${ids}; do
    gh api "repos/${GH_REPO}/actions/runs/${id}/jobs?per_page=100" \
      --jq '.jobs[] | {name, conclusion, html_url, completed_at}'
  done | jq -s .
}

if [[ "${1:-}" == "--jobs" ]]; then
  jobs="$(cat "${2:?--jobs needs a file}")"
else
  jobs="$(jobs_from_api)"
fi

# Every fuzz target that has ever run is a check of its own.
mapfile -t fuzz < <(jq -r --arg p "${FUZZ_PREFIX}" \
  '[.[] | select(.name | startswith($p)) | .name] | unique | .[]' <<<"${jobs}")
checks=("${CHECKS[@]}" "${fuzz[@]}")

failed=0
now="$(date -u +%s)"
for check in "${checks[@]}"; do
  latest="$(jq -c --arg n "${check}" \
    'first(.[] | select(.name == $n and (.conclusion == "success" or .conclusion == "failure"
      or .conclusion == "timed_out"))) // empty' <<<"${jobs}")"
  if [[ -z "${latest}" ]]; then
    echo "::warning::${check} has not run on develop yet; it cannot vouch for this release."
    continue
  fi
  conclusion="$(jq -r .conclusion <<<"${latest}")"
  url="$(jq -r .html_url <<<"${latest}")"
  if [[ "${conclusion}" != "success" ]]; then
    echo "::error::${check} failed in its latest run on develop (${url}). Fix it, then run" \
      "nightly.yml on develop by hand; a green run clears this gate."
    failed=1
    continue
  fi
  completed="$(jq -r '.completed_at // empty' <<<"${latest}")"
  if [[ -n "${completed}" ]]; then
    age_days=$(((now - $(date -u -d "${completed}" +%s)) / 86400))
    if ((age_days > STALE_DAYS)); then
      echo "::warning::${check} last passed ${age_days} days ago (${url})."
    fi
  fi
  echo "${check}: passed (${url})"
done
exit "${failed}"
