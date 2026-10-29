---
id: 01M63CFE5ZPFWFG98RXPQTMKGT
title: Single binary
vellum: 1
---

# Single binary

The single binary is the whole of Vellum: web app, API, MCP server,
renderer, search and background jobs, with an encrypted SQLite index.

## Install {#install}

Download the release for your platform from the project's releases
page, check its signature, and put it on your `PATH`.

```bash
cosign verify-blob --bundle vellum.sigstore.json vellum
chmod +x vellum && sudo mv vellum /usr/local/bin/
vellum --version
```

Every release is signed with Sigstore, built reproducibly and shipped
with a CycloneDX software bill of materials.

## Run {#run}

```bash
vellum serve --data-dir ~/vellum
```

Vellum listens on `127.0.0.1:7700` and creates the data folder on first
start. Open `http://127.0.0.1:7700` and create the first account; it
becomes the workspace owner. The same address serves the web app and
the API under `/api`, so there is nothing else to host.

To serve other people, give Vellum a public address and let it get a
certificate:

```bash
VELLUM_BIND=0.0.0.0:443 VELLUM_PUBLIC_URL=https://vellum.northstar.dev vellum serve --acme
```

## Run as a service {#service}

On Linux, run Vellum under systemd so it starts on boot and restarts on
failure:

```ini
[Unit]
Description=Vellum
After=network-online.target

[Service]
User=vellum
Environment=VELLUM_DATA_DIR=/var/lib/vellum
Environment=VELLUM_PUBLIC_URL=https://vellum.northstar.dev
ExecStart=/usr/local/bin/vellum serve
Restart=on-failure
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/vellum

[Install]
WantedBy=multi-user.target
```

## The data folder {#data-folder}

```text
vellum-data/
  .vellum/        # index.sqlite, trash, keys: never content
  drives/         # shared drives and My Drives
  teams/          # issues, one file each
  tasks/          # personal task lists
```

Back up the whole folder. Everything except `.vellum/index.sqlite` is
content; the index can be rebuilt from it with `vellum reindex`.
