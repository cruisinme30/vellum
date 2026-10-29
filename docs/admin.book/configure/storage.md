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

## Signing in to S3 {#s3-credentials}

The server reads the bucket from `VELLUM_BLOB_URL=s3://bucket[/prefix]`,
with `VELLUM_S3_REGION` and, for MinIO or R2, `VELLUM_S3_ENDPOINT`. It
signs in with the first of these that is set up, so on Kubernetes and AWS
no key has to be stored anywhere:

| Source | Set up with | Where |
| --- | --- | --- |
| Static keys | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | Anywhere |
| Web identity (IRSA) | `AWS_ROLE_ARN`, `AWS_WEB_IDENTITY_TOKEN_FILE` | EKS; any Kubernetes with MinIO STS (`AWS_ENDPOINT_URL_STS`) |
| Pod Identity or ECS | `AWS_CONTAINER_CREDENTIALS_FULL_URI` or `_RELATIVE_URI` | EKS Pod Identity, ECS tasks |
| Instance profile | Nothing (IMDSv2) | EC2, when no `VELLUM_S3_ENDPOINT` is set |

EKS sets the variables itself. With the Helm chart, leave
`blob.credentialsSecret` empty and annotate the service account:

```yaml
# northstar-values.yaml
blob:
  url: s3://northstar-vellum-content
  region: eu-west-1
serviceAccount:
  annotations:
    eks.amazonaws.com/role-arn: arn:aws:iam::000000000000:role/northstar-vellum
```

Temporary keys are refreshed five minutes before they expire. The role
needs `s3:GetObject`, `s3:PutObject`, `s3:DeleteObject` and
`s3:ListBucket` on the bucket.

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
