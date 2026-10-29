#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Upgrade drill (ADR 0022, D96): seed a stack on an older release, upgrade
# it in place to the new image, and check that nothing was lost and the new
# version serves.
#
#   deploy/drills/upgrade.sh FROM_IMAGE TO_IMAGE
# shellcheck source-path=SCRIPTDIR
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

from="$1" to="$2"
project="drill-upgrade" port=7811
out="$(mktemp -d)"
trap '"${stack}" down "${project}" >/dev/null 2>&1 || true' EXIT

echo "Starting ${from} and seeding it."
"${stack}" up "${from}" "${project}" "${port}"
seed "${port}" "${PAGES:-200}"
counts "${project}" >"${out}/before.txt"

echo "Upgrading in place to ${to}."
"${stack}" start-vellum "${project}" "${to}" "${port}"
counts "${project}" >"${out}/after.txt"
same_counts "upgrade ${from} -> ${to}" "${out}/before.txt" "${out}/after.txt"

echo "The upgraded server still accepts writes."
seed "${port}" 5
