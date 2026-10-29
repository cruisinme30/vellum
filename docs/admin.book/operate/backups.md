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
- Backup key: `VELLUM_BACKUP_KEY`, kept somewhere other than the server
- Index: optional; it can be rebuilt, but restoring it is faster than rebuilding a large workspace

:::

## The backup key {#key}

Backups are sealed with their own key, not the workspace key, so a copy
of the server's key folder cannot open them. Set `VELLUM_BACKUP_KEY` to
64 hexadecimal characters (for example `openssl rand -hex 32`) and store
the value in your password manager or secret store.

Without it, the server creates `.vellum/keys/backup.key` in the data
folder and warns at every start. A backup is then only as safe as the
server it came from: copy that file somewhere else, or set the variable.

The audit log is sealed with `VELLUM_ADMIN_KEY`, or with
`.vellum/keys/admin.key` when the variable is not set. Losing that key
makes the audit log unreadable, so back it up with the workspace keys. The
same goes for `.vellum/keys/index.key`, which holds the index key when
`VELLUM_INDEX_KEY` is not set. Backups never include the key folder.

## Taking backups {#take}

Turn on scheduled backups in the admin settings
(`PUT /api/v1/admin/settings`, under `backups`), or take one now with
`POST /api/v1/admin/backups`. Both need an administrator who signed in
within the last five minutes. On the server itself, `vellum admin backup`
takes one now, and `vellum admin backups` lists them. Each backup is a `.vbk` file of sealed
4 MiB chunks and a manifest holding every file's hash. It lands in
`.vellum/backups/` unless the settings name another folder. Point that
at a different disk or a mounted bucket.

The server checks every five minutes whether a backup is due. In the org
profile, also use Postgres continuous archiving for a recovery point of
zero.

## Restore drill {#drill}

The server runs a restore drill on the schedule in the backup settings,
and `POST /api/v1/admin/backups/drill` runs one now. A drill restores
the latest backup into scratch space, checks every file's hash and fails
when it takes longer than the 15-minute recovery time objective. Each
backup and drill is recorded in the audit log.

Once a quarter, also restore by hand onto a spare machine:

::: steps

1. **Copy** the latest `.vbk` file and the backup key to the spare machine
2. **Restore** with `vellum admin restore <file> --into <empty folder>`, with `VELLUM_BACKUP_KEY` set, then start `vellum serve` on that folder
3. **Open** a few known pages and an end-to-end encrypted space from the desktop app
4. **Record** the drill and its duration in the operations log

:::

A backup that has never been restored is a hope, not a backup.
