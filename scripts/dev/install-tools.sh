#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Installs the tools `cargo xtask fast` needs, at the versions CI uses, into
# $VELLUM_TOOLS_DIR (default ~/.local/bin). A tool already on PATH is left
# alone, so a second run takes a second. Linux x86_64 only; on a Mac, use
# Homebrew. A cloud environment's setup script runs it, so every cloud
# session starts with the tools (ADR 0048, D202):
#
#   curl -sSfL https://raw.githubusercontent.com/cruisinme30/vellum/develop/scripts/dev/install-tools.sh | bash
#
# Helm is not installed: its download host is not on the cloud network
# allowlist. scripts/ci/lint-infra.sh skips the Helm check locally.
set -euo pipefail

dest="${VELLUM_TOOLS_DIR:-${HOME}/.local/bin}"
mkdir -p "${dest}"
export PATH="${dest}:${PATH}"
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

if [[ "$(uname -sm)" != "Linux x86_64" ]]; then
  echo "install-tools.sh supports Linux x86_64 only; install the tools another way." >&2
  exit 1
fi

# Keep these in step with .github/workflows/ci.yml.
readonly VALE=3.25.1
readonly GITLEAKS=8.30.0
readonly SHELLCHECK=0.11.0
readonly LYCHEE=0.22.0
readonly NEXTEST=0.9.120
readonly DENY=0.19.0
readonly AUDIT=0.22.0
readonly MARKDOWNLINT=0.24.0
readonly MDBOOK=0.5.4

installed=()
failed=()

# have <tool>: succeeds when the tool is already on PATH.
have() { command -v "$1" >/dev/null 2>&1; }

# from_tarball <tool> <url> <path in archive>: unpacks one binary.
from_tarball() {
  local tool="$1" url="$2" member="$3" archive="${tmp}/$1.archive"
  curl -sSfL --retry 3 -o "${archive}" "${url}"
  mkdir -p "${tmp}/$1"
  tar -xf "${archive}" -C "${tmp}/$1"
  install -m 0755 "${tmp}/$1/${member}" "${dest}/${tool}"
}

# want <tool> <command...>: runs the install command unless the tool exists.
want() {
  local tool="$1"
  shift
  have "${tool}" && return 0
  if "$@" >"${tmp}/${tool}.log" 2>&1 && have "${tool}"; then
    installed+=("${tool}")
  else
    failed+=("${tool}")
    sed 's/^/    /' "${tmp}/${tool}.log" >&2
  fi
}

gh_release="https://github.com"
want vale from_tarball vale \
  "${gh_release}/errata-ai/vale/releases/download/v${VALE}/vale_${VALE}_Linux_64-bit.tar.gz" vale
want gitleaks from_tarball gitleaks \
  "${gh_release}/gitleaks/gitleaks/releases/download/v${GITLEAKS}/gitleaks_${GITLEAKS}_linux_x64.tar.gz" gitleaks
want shellcheck from_tarball shellcheck \
  "${gh_release}/koalaman/shellcheck/releases/download/v${SHELLCHECK}/shellcheck-v${SHELLCHECK}.linux.x86_64.tar.xz" \
  "shellcheck-v${SHELLCHECK}/shellcheck"
want lychee from_tarball lychee \
  "${gh_release}/lycheeverse/lychee/releases/download/lychee-v${LYCHEE}/lychee-x86_64-unknown-linux-gnu.tar.gz" lychee
want cargo-nextest from_tarball cargo-nextest \
  "${gh_release}/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST}/cargo-nextest-${NEXTEST}-x86_64-unknown-linux-gnu.tar.gz" \
  cargo-nextest
want cargo-deny from_tarball cargo-deny \
  "${gh_release}/EmbarkStudios/cargo-deny/releases/download/${DENY}/cargo-deny-${DENY}-x86_64-unknown-linux-musl.tar.gz" \
  "cargo-deny-${DENY}-x86_64-unknown-linux-musl/cargo-deny"
want cargo-audit from_tarball cargo-audit \
  "${gh_release}/rustsec/rustsec/releases/download/cargo-audit%2Fv${AUDIT}/cargo-audit-x86_64-unknown-linux-musl-v${AUDIT}.tgz" \
  "cargo-audit-x86_64-unknown-linux-musl-v${AUDIT}/cargo-audit"
# mdBook builds the website the UI layout checks audit.
want mdbook from_tarball mdbook \
  "${gh_release}/rust-lang/mdBook/releases/download/v${MDBOOK}/mdbook-v${MDBOOK}-x86_64-unknown-linux-gnu.tar.gz" \
  mdbook

# Python linters, pinned like CI. sqlfluff 4 rejects the migrations.
pip_tool() {
  if have uv; then
    uv tool install --quiet "$1"
  else
    python3 -m pip install --quiet --user --break-system-packages "$1"
  fi
}
want yamllint pip_tool yamllint==1.37.1
want sqlfluff pip_tool sqlfluff==3.4.2
want zizmor pip_tool zizmor==1.31.1
want reuse pip_tool reuse

want markdownlint-cli2 npm install --global --silent "markdownlint-cli2@${MARKDOWNLINT}"

# hadolint and kube-score, with the checksums CI checks. Piped from curl
# (a setup script), this file has no checkout beside it; fetch the helper.
if ! have hadolint || ! have kube-score; then
  infra="$(dirname "${BASH_SOURCE[0]:-.}")/../ci/install-infra-tools.sh"
  if [[ ! -f "${infra}" ]]; then
    infra="${tmp}/install-infra-tools.sh"
    curl -sSfL --retry 3 -o "${infra}" \
      "https://raw.githubusercontent.com/cruisinme30/vellum/develop/scripts/ci/install-infra-tools.sh"
  fi
  if bash "${infra}" "${dest}" >"${tmp}/infra.log" 2>&1; then
    installed+=(hadolint kube-score)
  else
    failed+=(hadolint kube-score)
    sed 's/^/    /' "${tmp}/infra.log" >&2
  fi
fi

if ((${#installed[@]} > 0)); then
  echo "Installed into ${dest}: ${installed[*]}"
fi
if ((${#failed[@]} > 0)); then
  echo "Could not install: ${failed[*]}. cargo xtask fast will stop until they are installed." >&2
  exit 1
fi
echo "Every Fast suite tool is installed."
