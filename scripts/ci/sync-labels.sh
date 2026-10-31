#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Creates or updates every label in .github/labels.yml. Needs gh with a
# token that may write issues, and the mikefarah yq.
set -euo pipefail

file="${1:-.github/labels.yml}"
count="$(yq 'length' "${file}")"
for ((i = 0; i < count; i++)); do
  name="$(yq ".[${i}].name" "${file}")"
  color="$(yq ".[${i}].color" "${file}")"
  description="$(yq ".[${i}].description" "${file}")"
  gh label create "${name}" --color "${color}" --description "${description}" \
    --force
done
echo "Synced ${count} labels."
