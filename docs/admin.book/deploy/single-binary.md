---
id: 01M63CFE5ZPFWFG98RXPQTMKGT
title: Single binary
vellum: 1
---

# Single binary

The single binary is the whole of Vellum: web app, API, MCP server,
renderer, search and background jobs, with an encrypted SQLite index.

## Install {#install}

Pick the channel for your platform. Every one installs the same signed
release binary, on x86-64 or ARM64 (a Raspberry Pi 3 or later on a
64-bit OS counts).

| Platform | Command |
| --- | --- |
| macOS or Linux, Homebrew | `brew install cruisinme30/vellum/vellum` |
| Windows | `winget install Vellum.Vellum` |
| Debian, Ubuntu | `sudo apt install ./vellum_<version>_amd64.deb` (or `_arm64`) |
| Fedora, RHEL, openSUSE | `sudo dnf install ./vellum-<version>-1.x86_64.rpm` (or `.aarch64`) |
| Any Linux or macOS | `curl -fsSL https://cruisinme30.github.io/vellum/install.sh \| sh` |

The `.deb` and `.rpm` files are on the release page. To install by hand,
download the binary for your platform, check its signature, and put it
on your `PATH`:

```bash
cosign verify-blob vellum-linux-amd64 --bundle vellum-linux-amd64.sigstore.json \
  --certificate-identity-regexp '^https://github.com/cruisinme30/vellum/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
chmod +x vellum-linux-amd64 && sudo mv vellum-linux-amd64 /usr/local/bin/vellum
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

The `.deb` and `.rpm` packages install a hardened systemd unit and a
`vellum` user, with data in `/var/lib/vellum`. Set the public address
and the key passphrase in `/etc/vellum/vellum.env`, then start it:

```bash
sudo systemctl enable --now vellum
```

With Homebrew, `brew services start vellum` does the same on macOS and
Linux, with data in Homebrew's `var/vellum`.

For a binary installed by hand, a minimal unit is:

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
