#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Checks every non-merge commit in BASE..HEAD for a Conventional Commits
# subject, a body after a blank line, and a DCO sign-off that matches
# the author.
# Usage: check-commits.sh <base-sha> <head-sha>
set -euo pipefail

base="${1:?base commit}"
head="${2:?head commit}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/ci/commit-rules.sh
source "${script_dir}/commit-rules.sh"

failures=0
while read -r sha; do
  [[ -z "${sha}" ]] && continue
  subject="$(git log -1 --format=%s "${sha}")"
  committer_email="$(git log -1 --format=%ce "${sha}")"
  author_name="$(git log -1 --format=%an "${sha}")"
  author_email="$(git log -1 --format=%ae "${sha}")"
  message="$(git log -1 --format=%B "${sha}")"
  # Squash merges made on GitHub carry the pull request title plus " (#N)";
  # their commits were checked when the pull request was open.
  if [[ "${committer_email}" == "noreply@github.com" ]]; then
    subject="${subject% (#[0-9]*)}"
  fi
  if ! check_subject "${subject}"; then
    echo "::error::${sha:0:12} subject is not a Conventional Commit: ${subject}"
    failures=$((failures + 1))
  fi
  if [[ "${author_email}" == *"[bot]@users.noreply.github.com" || "${committer_email}" == "noreply@github.com" ]]; then
    continue
  fi
  if ! check_body "${message}"; then
    echo "::error::${sha:0:12} needs a blank line after the subject and a body saying why (see .gitmessage)"
    failures=$((failures + 1))
  fi
  if ! check_signoff "${message}" "${author_name}" "${author_email}"; then
    echo "::error::${sha:0:12} lacks 'Signed-off-by: ${author_name} <${author_email}>' (use git commit -s)"
    failures=$((failures + 1))
  fi
done < <(git rev-list --no-merges "${base}..${head}")

if ((failures > 0)); then
  echo "${failures} commit rule violation(s)."
  exit 1
fi
echo "All commits follow the commit template and carry a DCO sign-off."
