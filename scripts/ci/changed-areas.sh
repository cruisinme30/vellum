#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Decides which parts of the Fast suite a change needs (ADR 0035, D139).
# Reads changed paths on standard input, one per line, and prints
# "rust=", "web=" and "desktop=" lines, each true or false. The Fast suite workflow and
# `cargo xtask fast` both use it, so they skip the same jobs.
#
# Rust is skipped only when every path is documentation that no crate
# embeds; the help books under docs/*.book are compiled in, so they count
# as Rust. Web runs only when web/ or the CI itself changes. Desktop runs
# when desktop/ or web/ changes, since the desktop app mirrors the web
# app's platform interface. With no paths (an unknown base), everything
# runs.
set -euo pipefail

rust=false
web=false
desktop=false
seen=false
while IFS= read -r path; do
  [[ -z "${path}" ]] && continue
  seen=true
  case "${path}" in
    .github/workflows/* | scripts/ci/*)
      rust=true
      web=true
      desktop=true
      ;;
    web/*)
      web=true
      desktop=true
      ;;
    desktop/*) desktop=true ;;
    docs/adr/* | site/* | .github/ISSUE_TEMPLATE/* | .github/pull_request_template.md) ;;
    */*) rust=true ;;
    *.md) ;;
    *) rust=true ;;
  esac
done

if [[ "${seen}" == false ]]; then
  rust=true
  web=true
  desktop=true
fi
echo "rust=${rust}"
echo "web=${web}"
echo "desktop=${desktop}"
