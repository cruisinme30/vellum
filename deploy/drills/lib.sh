#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Shared helpers for the release drills. Source it; do not run it.

drills="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${drills}/../.." && pwd)"
stack="${repo}/deploy/compose/stack.sh"
K6_IMAGE="${K6_IMAGE:-grafana/k6:2.3.0@sha256:2dd4756ec6e14e62778916e1eb850108e4b499304c02df56760fa0bdb0fba0b8}"

# seed PORT PAGES: creates and edits PAGES pages through the API.
seed() {
  docker run --rm --network host -v "${repo}/deploy/perf/k6:/k6:ro" \
    "${K6_IMAGE}" \
    run -q -e "BASE_URL=http://127.0.0.1:$1" -e "PAGES=$2" \
    -e "TOKEN=${TOKEN:-}" /k6/seed.js
}

# counts PROJECT: one "table rows" line per table in the index, then the
# number of object versions in the bucket. Two healthy copies of the same
# data print the same lines.
counts() {
  local project="$1"
  "${stack}" compose "${project}" exec -T postgres \
    psql -U vellum -d vellum -At -c "
    SELECT format('SELECT %L || '' '' || count(*) FROM %I.%I;',
                  table_name, table_schema, table_name)
    FROM information_schema.tables
    WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
    ORDER BY table_name" |
    "${stack}" compose "${project}" exec -T postgres psql -U vellum -d vellum \
      -At
  "${stack}" compose "${project}" run --rm --no-deps minio-init \
    "mc ls --recursive --versions local/vellum | wc -l |
     sed 's/^/blob-versions /'"
}

# same_counts LABEL FILE_A FILE_B: fails with a diff when they differ.
same_counts() {
  if diff -u "$2" "$3"; then
    echo "$1: counts match ($(wc -l <"$2") lines)."
  else
    echo "::error::$1: counts differ."
    return 1
  fi
}
