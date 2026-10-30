#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# The Fast suite's "UI layout (quick)" job, run locally by `cargo xtask fast`:
# builds the website, then runs the layout, focus and loading checks
# (ADR 0031) in Chromium at phone and laptop size, both CI shards at once,
# in cargo xtask fast only when VELLUM_FAST_ALL_TESTS=1 (the integrator's run).
# The integrator runs it on every merged batch, so a UI regression is found
# before the batch's CI run. Needs mdbook (scripts/dev/install-tools.sh) and
# web/node_modules (pnpm install). Extra arguments go to Playwright, such as
# `-g "app shell"` for a focused run.
#
# Environment: UI_WORKERS (default 4). UI_CHROMIUM_PATH points Playwright at
# a preinstalled Chromium; when unset, a cloud sandbox's /opt/pw-browsers
# Chromium is used if there is one.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
site="${root}/target/ui-site"

if ! command -v "${MDBOOK:-mdbook}" >/dev/null 2>&1; then
  echo "mdbook is not installed; run scripts/dev/install-tools.sh" >&2
  exit 1
fi
SITE_VERSIONS=0 "${root}/scripts/ci/build-site.sh" "${site}" >/dev/null

cd "${root}/web"
if [[ -z "${UI_CHROMIUM_PATH:-}" ]]; then
  for candidate in /opt/pw-browsers/chromium-*/chrome-linux64/chrome; do
    if [[ -x "${candidate}" ]]; then
      export UI_CHROMIUM_PATH="${candidate}"
      break
    fi
  done
fi

UI_SCOPE=quick UI_BROWSERS=chromium UI_VIEWPORTS=phone,laptop \
  UI_SITE_DIR="${site}" \
  exec pnpm exec playwright test e2e/ui --workers="${UI_WORKERS:-4}" \
  --reporter=line "$@"
