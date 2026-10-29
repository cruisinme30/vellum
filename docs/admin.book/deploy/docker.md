---
id: 01M63CFE60EG0WHG200X6EFW01
title: Docker
vellum: 1
---

# Docker

The container image holds the same single binary on a minimal base,
running as a non-root user.

```bash
docker run -d --name vellum \
  -p 7700:7700 \
  -v vellum-data:/data \
  -e VELLUM_DATA_DIR=/data \
  -e VELLUM_BIND=0.0.0.0:7700 \
  -e VELLUM_PUBLIC_URL=https://vellum.northstar.dev \
  ghcr.io/cruisinme30/vellum:1
```

Put a reverse proxy that terminates TLS in front, or pass `--acme` and
publish port 443 instead.

## Health checks {#health}

::: facts

- `/healthz`: the process is alive
- `/readyz`: the process can serve requests (index open, storage reachable)

:::

Both return `200` or `503`, and say why in the body. Orchestrators
should restart on `/healthz` and route on `/readyz`.
