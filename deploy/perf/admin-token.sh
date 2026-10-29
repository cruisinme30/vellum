#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Prints a one-day API token with the admin scope for a CI stack, so that
# `vellum budgets` can read GET /api/v1/admin/routes (ADR 0022, D254). On
# an empty server it first sets up the administrator; afterwards it signs
# in with the same details the k6 suite uses (EMAIL, PASSWORD).
#
#   deploy/perf/admin-token.sh BASE_URL
set -euo pipefail

base="${1:?usage: admin-token.sh BASE_URL}"
email="${EMAIL:-load@northstar.dev}"
password="${PASSWORD:-northstar-load-test}"
jar="$(mktemp)"
trap 'rm -f "${jar}"' EXIT

# The $names are jq variables, not shell ones.
# shellcheck disable=SC2016
body() { jq -cn --arg email "${email}" --arg password "${password}" "$1"; }

# 200 makes the administrator; 409 means one exists already.
# shellcheck disable=SC2016
status="$(curl -sS -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' \
  -d "$(body '{email: $email, name: "Northstar load", password: $password}')" \
  "${base}/api/v1/setup")"
if [[ "${status}" != 200 && "${status}" != 409 ]]; then
  echo "setup answered ${status}" >&2
  exit 1
fi

# shellcheck disable=SC2016
curl -sSf -c "${jar}" -b "${jar}" -o /dev/null -H 'Content-Type: application/json' \
  -d "$(body '{email: $email, password: $password}')" "${base}/auth/sign-in"
csrf="$(curl -sSf -c "${jar}" -b "${jar}" "${base}/api/csrf" | jq -r .token)"
curl -sSf -c "${jar}" -b "${jar}" -H 'Content-Type: application/json' \
  -H "x-csrf-token: ${csrf}" \
  -d '{"name": "budgets", "scopes": ["read", "admin"], "expiresInDays": 1}' \
  "${base}/api/v1/me/tokens" | jq -er .token
