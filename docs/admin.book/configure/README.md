---
id: 01M63CFE61Y6QWWCCYRBD6DCTY
title: Configuration
vellum: 1
---

# Configuration

Vellum reads its settings in three layers, each overriding the one
before: built-in defaults, a TOML file, then environment variables.
Point Vellum at the file with `VELLUM_CONFIG` or `--config`. Unknown
keys in the file are an error, so a typo never passes silently.

```toml
# /etc/vellum/vellum.toml
data_dir = "/var/lib/vellum"
bind = "0.0.0.0:7700"
public_url = "https://vellum.northstar.dev"
profile = "team"
database_url = "postgres://vellum@db/vellum"
```

## Server settings {#server}

| File key | Variable | Default | Meaning |
| --- | --- | --- | --- |
| (none) | `VELLUM_CONFIG` | unset | Path of the TOML file |
| `data_dir` | `VELLUM_DATA_DIR` | `vellum-data` | Folder for `.vellum/`, drives, teams and tasks |
| `bind` | `VELLUM_BIND` | `127.0.0.1:7700` | Address to listen on |
| `public_url` | `VELLUM_PUBLIC_URL` | derived from `bind` | External address, for links and sign-in |
| `profile` | `VELLUM_PROFILE` | `single` | `single`, `team` or `org` |
| `database_url` | `VELLUM_DATABASE_URL` | SQLite in `data_dir` | Postgres for the index |

## Logs and telemetry {#telemetry}

| Variable | Default | Meaning |
| --- | --- | --- |
| `VELLUM_LOG` | `info` | Log filter, such as `info,vellum_render=debug` |
| `VELLUM_LOG_FORMAT` | `json` | `json` or `pretty` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset | Send traces, metrics and logs to an OpenTelemetry collector |

See [Observability](../operate/observability.md) for what Vellum
records.

## Secrets {#secrets}

Pass secrets such as database passwords through environment variables
or files mounted by your orchestrator, never in the TOML file you
commit. Vellum never writes a secret to its logs; the configuration's
debug output shows `<redacted>` in their place.

## Secrets in pages {#secret-scanning}

Vellum scans every page as it is saved, whether a person, live editing
or a model wrote it, for API keys, tokens and private keys. Text files
attached to pages (`.txt`, `.csv`, `.env` and the like) and collection
rows, including CSV imports, are scanned the same way. Set
what happens in **Admin → Settings → Security**:

::: facts

- Warn, and save (the default): the save goes through, the editor shows a banner naming the kind and line, and the audit log records it
- Block the save: the save is refused until the secret is removed; models get a tool error
- Don't scan: nothing is checked

:::

**Allowed secrets** takes one entry per line: a kind such as `jwt` skips
that kind everywhere, and any other text skips secrets that contain it,
such as a documented example key. A line holding `vellum:allow-secret` is
never scanned. The secret itself is never written to the audit log, the
server log or an error.

## Workspace settings {#workspace}

Everything else, from sign-in to quotas, is set in the app under
**Admin**, is stored in the index and is recorded in the audit log.
Changing an admin setting asks you to sign in again.
