#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Fails when BASE...HEAD changes more than the limit, not counting files
# marked linguist-generated in .gitattributes.
# Usage: check-pr-size.sh <base-sha> <head-sha> [limit]
set -euo pipefail

base="${1:?base commit}"
head="${2:?head commit}"
limit="${3:-400}"

total=0
while IFS=$'\t' read -r added deleted path; do
  [[ "${added}" == "-" ]] && continue # binary file
  if git check-attr linguist-generated -- "${path}" | grep -q ': true$'; then
    continue
  fi
  total=$((total + added + deleted))
done < <(git diff --numstat "${base}...${head}")

echo "Changed lines (excluding generated files): ${total} (limit ${limit})"
if ((total > limit)); then
  echo "::error::This pull request changes ${total} lines; split it, or have a maintainer apply the size/exempt label."
  exit 1
fi
