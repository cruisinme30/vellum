#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Checks that a clamd scans uploads as large as Vellum accepts (#298).
#
#   deploy/clamav/smoke.sh [host] [port]      # default 127.0.0.1 3310
#
# Streams a clean 30 MB file through INSTREAM (clamd's default 25 MB
# StreamMaxLength refuses it), then the EICAR test string, which must be
# reported as found. Exits non-zero on any other reply.
set -euo pipefail
trap '' PIPE # report clamd's reply instead of dying on a closed socket

host="${1:-127.0.0.1}"
port="${2:-3310}"

# instream <file>: prints clamd's reply for the file's bytes.
instream() {
  local size
  size="$(stat -c %s "$1")"
  exec 3<>"/dev/tcp/${host}/${port}"
  {
    printf 'zINSTREAM\0'
    # One chunk: a 4-byte big-endian length, the bytes, then a zero length.
    printf '%b' "$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' \
      $(((size >> 24) & 255)) $(((size >> 16) & 255)) \
      $(((size >> 8) & 255)) $((size & 255)))"
    cat "$1"
    printf '\0\0\0\0'
  } >&3 2>/dev/null || true # clamd hangs up early on an oversized stream
  tr -d '\0' <&3
  exec 3>&-
}

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

head -c $((30 * 1024 * 1024)) /dev/zero | tr '\0' 'a' >"${work}/clean.bin"
reply="$(instream "${work}/clean.bin")"
echo "30 MB clean file: ${reply}"
if [[ "${reply}" != "stream: OK" ]]; then
  echo "::error::clamd did not scan a 30 MB file; set StreamMaxLength to at" \
    "least 100M." >&2
  exit 1
fi

# The EICAR test string, split so this script is not itself flagged.
# shellcheck disable=SC2016 # literal text, nothing to expand
printf '%s%s' 'X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-' \
  'ANTIVIRUS-TEST-FILE!$H+H*' >"${work}/eicar.txt"
reply="$(instream "${work}/eicar.txt")"
echo "EICAR test file: ${reply}"
if [[ "${reply}" != *" FOUND" ]]; then
  echo "::error::clamd did not flag the EICAR test file." >&2
  exit 1
fi
echo "clamd scans 30 MB uploads and detects the test signature."
