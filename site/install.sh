#!/bin/sh
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Installs the Vellum binary on Linux or macOS (ADR 0022, D250):
#
#   curl -fsSL https://cruisinme30.github.io/vellum/install.sh | sh
#
# It downloads the release binary for this machine from GitHub, checks it
# against the release's SHA256SUMS (and its Sigstore signature when cosign
# is installed), and puts it in ~/.local/bin, or /usr/local/bin as root.
# Settings, all optional:
#   VELLUM_VERSION      a release such as 1.2.3 (default: the latest)
#   VELLUM_INSTALL_DIR  where to put the binary
set -eu

repo="cruisinme30/vellum"

fail() {
  echo "install.sh: $*" >&2
  exit 1
}

case "$(uname -s)" in
  Linux)
    case "$(uname -m)" in
      x86_64 | amd64) file="vellum-linux-amd64" ;;
      aarch64 | arm64) file="vellum-linux-arm64" ;;
      armv7l | armv6l) fail \
        "32-bit ARM is not supported; install a 64-bit OS" \
        "(Raspberry Pi 3 or later)" ;;
      *) fail "no release binary for $(uname -m)" ;;
    esac
    ;;
  Darwin) file="vellum-macos-universal" ;;
  *) fail "use winget on Windows: winget install Vellum.Vellum" ;;
esac

if [ -n "${VELLUM_VERSION:-}" ]; then
  base="https://github.com/${repo}/releases/download/v${VELLUM_VERSION#v}"
else
  base="https://github.com/${repo}/releases/latest/download"
fi
if [ -n "${VELLUM_INSTALL_DIR:-}" ]; then
  dir="${VELLUM_INSTALL_DIR}"
elif [ "$(id -u)" = 0 ]; then
  dir="/usr/local/bin"
else
  dir="${HOME}/.local/bin"
fi

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q --https-only -O "$2" "$1"; }
else
  fail "needs curl or wget"
fi

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT INT TERM
echo "Downloading ${file} from ${base}"
fetch "${base}/${file}" "${work}/${file}"
fetch "${base}/SHA256SUMS" "${work}/SHA256SUMS"

expected="$(awk -v f="${file}" '$2 == f || $2 == "*" f { print $1 }' \
  "${work}/SHA256SUMS")"
[ -n "${expected}" ] || fail "SHA256SUMS does not list ${file}"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${work}/${file}" | awk '{ print $1 }')"
else
  actual="$(shasum -a 256 "${work}/${file}" | awk '{ print $1 }')"
fi
[ "${actual}" = "${expected}" ] || fail "checksum mismatch for ${file}"

if command -v cosign >/dev/null 2>&1; then
  fetch "${base}/${file}.sigstore.json" "${work}/${file}.sigstore.json"
  cosign verify-blob "${work}/${file}" --bundle \
    "${work}/${file}.sigstore.json" \
    --certificate-identity-regexp "^https://github.com/${repo}/" \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    >/dev/null 2>&1 ||
    fail "the Sigstore signature of ${file} does not verify"
  echo "Signature verified"
fi

mkdir -p "${dir}"
chmod 0755 "${work}/${file}"
mv "${work}/${file}" "${dir}/vellum"
echo "Installed ${dir}/vellum"
case ":${PATH}:" in
  *":${dir}:"*) ;;
  *) echo "Add ${dir} to your PATH to run it as: vellum serve" ;;
esac
echo "Start it with: vellum serve   (then open http://127.0.0.1:7700)"
