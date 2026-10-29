---
id: 01M63CFE62GAV995KXV75MBAWW
title: Upgrades
vellum: 1
---

# Upgrades

Vellum follows semantic versioning. Upgrades within a major version
never need manual steps; database migrations run on start and are
tested from an empty database and from the previous release.

## Single binary and Docker {#single}

1. Take a backup with `vellum backup`.
2. Replace the binary or image tag.
3. Restart. Vellum migrates the index and reports in its log when it is ready.

## Helm {#helm}

```bash
helm upgrade vellum oci://ghcr.io/cruisinme30/charts/vellum --version 1.1.0 --values northstar-values.yaml
```

The chart runs migrations as a pre-upgrade job and rolls out as a canary
(10%, 50%, 100%) with automatic rollback.

## Rolling back {#rollback}

Within a minor version, roll back by deploying the previous version.
Across minor versions, migrations can be one-way: restore the backup you
took before upgrading.

## Release notes {#notes}

Each release's notes list new features, fixes, migrations and any
setting whose default changed. The help books shipped in the binary
match its version; run `vellum docs` to read them.
