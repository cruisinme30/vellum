#!/bin/sh
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Before removal: stop the service. An upgrade keeps it running (the new
# package's postinstall restarts it). The data folder is always kept.
set -eu

case "${1:-}" in
  upgrade | 1) exit 0 ;; # deb passes "upgrade"; rpm passes 1 on upgrade
esac
if [ -d /run/systemd/system ]; then
  systemctl disable --now vellum >/dev/null 2>&1 || true
fi
