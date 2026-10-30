#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Merges the line coverage of the Rust coverage shards (ADR 0076, D324)
# and fails when the total is under the floor. Each shard ran a quarter
# of the tests over the whole instrumented workspace, so a line counts as
# covered when any shard ran it; hit counts add up.
#
#   scripts/ci/merge-coverage.sh MIN_PERCENT OUT.info SHARD.info...
#
# Reads only the SF and DA records of the lcov files and writes SF, DA,
# LF and LH records, which is all the line floor and the uploaded report
# need.
set -euo pipefail

if (($# < 3)); then
  echo "usage: $0 MIN_PERCENT OUT.info SHARD.info..." >&2
  exit 2
fi
min="$1"
out="$2"
shift 2

# One "file<TAB>line<TAB>hits" row per DA record, sorted by file and line.
awk -v OFS='\t' '
  /^SF:/ { file = substr($0, 4); next }
  /^DA:/ { split(substr($0, 4), da, ","); print file, da[1], da[2] }
' "$@" | LC_ALL=C sort -t "$(printf '\t')" -k1,1 -k2,2n |
  awk -F '\t' -v min="${min}" -v out="${out}" '
    function line_done() {
      if (have) {
        printf "DA:%s,%s\n", line, hits > out
        found++
        if (hits > 0) hit++
      }
      have = 0
    }
    function file_done() {
      line_done()
      if (file != "") {
        printf "LF:%d\nLH:%d\nend_of_record\n", found, hit > out
        total_found += found
        total_hit += hit
      }
      found = 0
      hit = 0
    }
    $1 != file { file_done(); file = $1; printf "SF:%s\n", file > out }
    $2 != line || !have { line_done(); line = $2; hits = 0; have = 1 }
    { hits += $3 }
    END {
      file_done()
      if (total_found == 0) {
        print "::error::no line coverage records in the shard reports"
        exit 1
      }
      percent = 100 * total_hit / total_found
      printf "Line coverage: %d of %d lines, %.2f %%\n",
        total_hit, total_found, percent
      if (percent < min) {
        printf "::error::line coverage %.2f %% is under %s %%\n",
          percent, min
        exit 1
      }
    }
  '
