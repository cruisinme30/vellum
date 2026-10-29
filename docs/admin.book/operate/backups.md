---
id: 01M63CFE62665T0RXR3CBXSZQ8
title: Backups and restore drills
vellum: 1
---

# Backups and restore drills

## What to back up {#what}

::: facts

- Content: the data folder or object storage bucket
- Keys: the workspace key, or access to the KMS that holds it
- Index: optional; it can be rebuilt, but restoring it is faster than rebuilding a large workspace

:::

## Taking backups {#take}

```bash
vellum backup --to s3://northstar-backups/vellum
```

`vellum backup` writes an encrypted, consistent snapshot of content and
index. Schedule it from cron or a Kubernetes CronJob. In the org
profile, also use Postgres continuous archiving for a recovery point of
zero.

## Restore drill {#drill}

Restore into a scratch deployment once a month and check it:

::: steps

1. **Restore** with `vellum restore --from s3://northstar-backups/vellum --data-dir /tmp/drill`
2. **Verify** with `vellum doctor --data-dir /tmp/drill`, which checks every file against the index and every key unwraps
3. **Open** a few known pages and an end-to-end encrypted space from the desktop app
4. **Record** the drill and its duration in the operations log

:::

A backup that has never been restored is a hope, not a backup.
