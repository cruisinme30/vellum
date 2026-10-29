---
id: 01M63CFE609BZ1WAAG29DH7PFW
title: Docker Compose
vellum: 1
---

# Docker Compose

The **team** profile runs Vellum with Postgres for the index, MinIO for
content and Kroki for diagrams. The repository's `deploy/` folder has
a ready-made compose file and an example environment file.

```yaml
services:
  vellum:
    image: ghcr.io/cruisinme30/vellum:1
    environment:
      VELLUM_PROFILE: team
      VELLUM_BIND: 0.0.0.0:7700
      VELLUM_PUBLIC_URL: https://vellum.northstar.dev
      VELLUM_DATABASE_URL: postgres://vellum:${POSTGRES_PASSWORD}@postgres/vellum
      VELLUM_STORAGE: s3://vellum-content?endpoint=http://minio:9000
      VELLUM_KROKI_URL: http://kroki:8000
    ports: ["7700:7700"]
    depends_on: [postgres, minio, kroki]
  postgres:
    image: postgres:17
    environment:
      POSTGRES_USER: vellum
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes: [pg:/var/lib/postgresql/data]
  minio:
    image: minio/minio
    command: server /data
    volumes: [blobs:/data]
  kroki:
    image: yuzutech/kroki
volumes:
  pg:
  blobs:
```

```bash
cp deploy/.env.example .env   # set POSTGRES_PASSWORD and the MinIO keys
docker compose up -d
```

## Kroki

Kroki draws D2, PlantUML, Graphviz, Excalidraw and draw.io diagrams.
Vellum calls it only from the server, with a timeout and a circuit
breaker; if Kroki is down, pages show the last drawing and the source.
Mermaid and architecture diagrams are drawn inside Vellum and never need
Kroki. Leave Kroki out if your users do not need those types.

## Virus scanning {#clamav}

The compose stack runs ClamAV next to Vellum, and the Helm chart runs two
copies of it. Every upload, up to the 100 MiB limit, is streamed to it
before it is stored. If ClamAV is down, uploads are refused rather than
stored unscanned. ClamAV needs about 1.5 GiB of memory and fetches new
signatures every two hours.

Point the upload scanner at it in **Admin → Security → Uploads**: the
address is `clamav:3310` in compose and `<release>-vellum-clamav:3310` in
Kubernetes. Both also pass it to Vellum as `VELLUM_CLAMD_ADDRESS`.

If you run your own ClamAV, set `StreamMaxLength 100M` in `clamd.conf`.
Many packages ship `25M`, which refuses every upload between 25 and
100 MiB. To check a scanner, run `deploy/clamav/smoke.sh <host> 3310`. It
streams a 30 MB file and the EICAR test file through it.

## Redis {#redis}

Redis is optional in every profile. When `VELLUM_REDIS_URL` is set,
Vellum uses it for caches and job queues; without it, Vellum keeps
caches in memory and queues jobs in Postgres.
