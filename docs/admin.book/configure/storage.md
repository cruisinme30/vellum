---
id: 01M63CFE615720F85XEF8Q8285
title: Storage backends
vellum: 1
---

# Storage backends

Content (pages, assets, attachments) lives in a storage backend; the
index lives in SQLite or Postgres. Each shared drive can use a different
backend, so one team can keep its docs in Git while the rest live in
S3.

| Backend | Address | Notes |
| --- | --- | --- |
| Local folder | `file:///var/lib/vellum` | The default; fastest |
| S3-compatible | `s3://bucket?endpoint=…` | AWS S3, MinIO, R2, GCS in S3 mode |
| Google Drive | `gdrive://folder-id` | Eventually consistent; always merged |
| Dropbox | `dropbox:///Vellum` | Eventually consistent; always merged |
| OneDrive | `onedrive:///Vellum` | Eventually consistent; always merged |
| Git | `git+ssh://git@github.com/northstar/docs.git` | A commit per save batch; pull and rebase through the merge |

## Quotas and retention {#quotas}

In **Admin → Drives**, set per drive:

::: facts

- Quota: the most the drive may hold
- Version retention: how long to keep old versions (default: forever)
- Trash retention: how long deleted items stay in the trash (default: 30 days)

:::

## Changing backend {#migrate}

`vellum migrate-storage --drive product --to s3://northstar-product`
copies every object, verifies checksums, switches the drive and leaves
the old copy in place until you delete it.

## External changes {#external}

Vellum watches every backend for changes made outside it and merges
them three ways. It never overwrites a change it did not see: a clash
becomes a conflict copy next to the page. See
[External edits](../../user.book/drive/external-edits.md) in the user
guide.
