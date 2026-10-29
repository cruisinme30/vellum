#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Refreshes the UI suite's visual baselines (ADR 0031, D125) inside the
# pinned Playwright container, the only place they are compared, so they
# match CI pixel for pixel on any host. Needs Docker. Review the changed
# PNGs in web/e2e/__screenshots__/ before committing them.
#
# Usage: scripts/ci/ui-baselines.sh [extra playwright args]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="$(sed -n 's/.*"@playwright\/test": "^\{0,1\}\([0-9.]*\)".*/\1/p' "${root}/web/package.json")"
image="mcr.microsoft.com/playwright:v${version}-noble"

# The container installs its own node_modules into a named volume, so the
# host's (perhaps macOS) packages are left alone.
docker run --rm --ipc=host \
  -v "${root}:/repo" -v vellum-ui-node-modules:/repo/web/node_modules \
  -w /repo/web -e HOME=/tmp -e CI= -e UI_VISUAL=1 -e UI_BROWSERS=chromium \
  "${image}" bash -c 'corepack enable && pnpm install --frozen-lockfile &&
    pnpm exec playwright test e2e/ui/visual.spec.ts --update-snapshots "$@"' bash "$@"
