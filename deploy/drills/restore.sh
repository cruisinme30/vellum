#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Restore drill (ADR 0022, D96): seed a stack, back up its database and
# bucket, restore both into a scratch stack that shares only the KEK, and
# check that the counts match and the restored server serves.
#
#   deploy/drills/restore.sh IMAGE
# shellcheck source-path=SCRIPTDIR
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

image="$1"
source_project="drill-source" scratch_project="drill-restore"
out="$(mktemp -d)"
backup="${out}/backup"
mkdir -p "${backup}/bucket"
chmod 0777 "${backup}/bucket"
cleanup() {
  "${stack}" down "${source_project}" >/dev/null 2>&1 || true
  "${stack}" down "${scratch_project}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "Seeding the source stack."
"${stack}" up "${image}" "${source_project}" 7821
seed 7821 "${PAGES:-200}"
counts "${source_project}" >"${out}/source.txt"

echo "Backing up the database and the bucket."
"${stack}" compose "${source_project}" exec -T postgres \
  pg_dump -U vellum -d vellum --format=custom >"${backup}/vellum.dump"
"${stack}" compose "${source_project}" run --rm --no-deps -v "${backup}/bucket:/backup" minio-init \
  "mc mirror --quiet local/vellum /backup"
kek="${STACK_STATE:-${RUNNER_TEMP:-/tmp}/vellum-stacks}/${source_project}.kek"

echo "Restoring into a scratch stack."
"${stack}" up-deps "${scratch_project}" 7822 "${kek}"
"${stack}" compose "${scratch_project}" exec -T postgres \
  pg_restore -U vellum -d vellum --no-owner --exit-on-error <"${backup}/vellum.dump"
"${stack}" compose "${scratch_project}" run --rm --no-deps -v "${backup}/bucket:/backup:ro" minio-init \
  "mc mirror --quiet /backup local/vellum"
"${stack}" start-vellum "${scratch_project}" "${image}" 7822
counts "${scratch_project}" >"${out}/restored.txt"

# Mirroring copies the latest version of each object only, so compare the
# index tables exactly and require at least one object per source object.
grep -v '^blob-versions ' "${out}/source.txt" >"${out}/source-index.txt"
grep -v '^blob-versions ' "${out}/restored.txt" >"${out}/restored-index.txt"
same_counts "restore of ${image}" "${out}/source-index.txt" "${out}/restored-index.txt"
seed 7822 5
