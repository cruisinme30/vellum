#!/bin/sh
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# After install or upgrade: create the vellum user, give it the data
# folder and reload systemd. The service is not enabled or started here.
set -eu

if command -v systemd-sysusers >/dev/null 2>&1; then
  systemd-sysusers /usr/lib/sysusers.d/vellum.conf
elif ! getent passwd vellum >/dev/null 2>&1; then
  useradd --system --home-dir /var/lib/vellum --no-create-home \
    --shell /usr/sbin/nologin --user-group vellum
fi
chown vellum:vellum /var/lib/vellum
chown root:vellum /etc/vellum/vellum.env
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
  # An upgrade restarts a running server so it picks up the new binary.
  if systemctl is-active --quiet vellum; then
    systemctl try-restart vellum || true
  fi
fi
