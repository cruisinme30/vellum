#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Installs the tools `cargo xtask fast` needs, at the versions CI uses, into
# $VELLUM_TOOLS_DIR (default ~/.local/bin). A tool already on PATH is left
# alone, so a second run takes a second. Linux x86_64 and aarch64; on a Mac,
# use the gate's container (scripts/dev/gate.sh, ADR 0073), which runs this
# script to build its image. Every download is checked against the SHA-256
# its release publishes. A cloud environment's setup script runs it, so
# every cloud session starts with the tools (ADR 0048, D202):
#
# curl -sSfL
# https://raw.githubusercontent.com/cruisinme30/vellum/develop/scripts/dev/install-tools.sh
# | bash
#
# Helm is installed only with VELLUM_INSTALL_HELM=1: its download host is
# not on the cloud network allowlist. scripts/ci/lint-infra.sh skips the
# Helm check locally when it is missing.
set -euo pipefail

dest="${VELLUM_TOOLS_DIR:-${HOME}/.local/bin}"
mkdir -p "${dest}"
export PATH="${dest}:${PATH}"
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
raw="https://raw.githubusercontent.com/cruisinme30/vellum/develop"
here="$(dirname "${BASH_SOURCE[0]:-.}")"

case "$(uname -sm)" in
  "Linux x86_64") arch=x86_64 ;;
  "Linux aarch64" | "Linux arm64") arch=aarch64 ;;
  *)
    echo "install-tools.sh supports Linux x86_64 and aarch64; on a Mac, use scripts/dev/gate.sh." >&2
    exit 1
    ;;
esac

# Keep these in step with .github/workflows/ci.yml.
readonly VALE=3.25.1
readonly GITLEAKS=8.30.0
readonly SHELLCHECK=0.11.0
readonly LYCHEE=0.22.0
readonly NEXTEST=0.9.120
readonly DENY=0.19.0
readonly AUDIT=0.22.0
readonly MARKDOWNLINT=0.24.0 # markdownlint-cli2-action v25.0.0
readonly MDBOOK=0.5.4
# full.yml's coverage job takes the latest from taiki-e/install-action;
# pinned here, so a new release is a reviewed change.
readonly LLVM_COV=0.9.1
readonly HELM=3.19.0 # azure/setup-helm in ci.yml
readonly REUSE=6.2.0 # fsfe/reuse-action v6.0.0 runs the fsfe/reuse:6 image

gh_release="https://github.com"
# One line per tool and architecture: tool, arch, URL, SHA-256 of the
# download (the `digest` GitHub lists for the release asset), and the
# binary's path inside the archive ("-" for a bare binary).
downloads="$(
  cat <<EOF
vale x86_64 ${gh_release}/errata-ai/vale/releases/download/v${VALE}/vale_${VALE}_Linux_64-bit.tar.gz 4bf5c5176b682b1da0830f471584d3494f1c2e27ee3029e1ad6f6661d4962182 vale
vale aarch64 ${gh_release}/errata-ai/vale/releases/download/v${VALE}/vale_${VALE}_Linux_arm64.tar.gz 2a69755b68118c612c455fc0c5ea1b9438519aa1c852e3fb303b3ad97141bbb0 vale
gitleaks x86_64 ${gh_release}/gitleaks/gitleaks/releases/download/v${GITLEAKS}/gitleaks_${GITLEAKS}_linux_x64.tar.gz 79a3ab579b53f71efd634f3aaf7e04a0fa0cf206b7ed434638d1547a2470a66e gitleaks
gitleaks aarch64 ${gh_release}/gitleaks/gitleaks/releases/download/v${GITLEAKS}/gitleaks_${GITLEAKS}_linux_arm64.tar.gz b4cbbb6ddf7d1b2a603088cd03a4e3f7ce48ee7fd449b51f7de6ee2906f5fa2f gitleaks
shellcheck x86_64 ${gh_release}/koalaman/shellcheck/releases/download/v${SHELLCHECK}/shellcheck-v${SHELLCHECK}.linux.x86_64.tar.xz 8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198 shellcheck-v${SHELLCHECK}/shellcheck
shellcheck aarch64 ${gh_release}/koalaman/shellcheck/releases/download/v${SHELLCHECK}/shellcheck-v${SHELLCHECK}.linux.aarch64.tar.xz 12b331c1d2db6b9eb13cfca64306b1b157a86eb69db83023e261eaa7e7c14588 shellcheck-v${SHELLCHECK}/shellcheck
lychee x86_64 ${gh_release}/lycheeverse/lychee/releases/download/lychee-v${LYCHEE}/lychee-x86_64-unknown-linux-gnu.tar.gz d004215a789db155779d3263e6e18ad424879b18eeddb00745efb8d9b2bd8891 lychee
lychee aarch64 ${gh_release}/lycheeverse/lychee/releases/download/lychee-v${LYCHEE}/lychee-aarch64-unknown-linux-gnu.tar.gz 29901e5df983fe2ff961ee9660c4ef3b4a9eae85d4776d0dc89581332e82d6db lychee
cargo-nextest x86_64 ${gh_release}/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST}/cargo-nextest-${NEXTEST}-x86_64-unknown-linux-gnu.tar.gz a5b1c12500c47e27af4baf533c917bf1b38e9bf2e6ffb063dfa1de6e75aa8726 cargo-nextest
cargo-nextest aarch64 ${gh_release}/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST}/cargo-nextest-${NEXTEST}-aarch64-unknown-linux-gnu.tar.gz 5e13751733a1fc4d26984ad5e1bce10d057d95299b02ed3ac96877b7288c8feb cargo-nextest
cargo-deny x86_64 ${gh_release}/EmbarkStudios/cargo-deny/releases/download/${DENY}/cargo-deny-${DENY}-x86_64-unknown-linux-musl.tar.gz 0e8c2aa59128612c90d9e09c02204e912f29a5b8d9a64671b94608cbe09e064f cargo-deny-${DENY}-x86_64-unknown-linux-musl/cargo-deny
cargo-deny aarch64 ${gh_release}/EmbarkStudios/cargo-deny/releases/download/${DENY}/cargo-deny-${DENY}-aarch64-unknown-linux-musl.tar.gz 2b3567a60b7491c159d1cef8b7d8479d1ad2a31e29ef49462634ad4552fcc77d cargo-deny-${DENY}-aarch64-unknown-linux-musl/cargo-deny
cargo-audit x86_64 ${gh_release}/rustsec/rustsec/releases/download/cargo-audit%2Fv${AUDIT}/cargo-audit-x86_64-unknown-linux-musl-v${AUDIT}.tgz 0ea7a15f67d5007af5bd18f89be38a90ebd004f149dc36ebfab0aedba24cbb4f cargo-audit-x86_64-unknown-linux-musl-v${AUDIT}/cargo-audit
cargo-audit aarch64 ${gh_release}/rustsec/rustsec/releases/download/cargo-audit%2Fv${AUDIT}/cargo-audit-aarch64-unknown-linux-gnu-v${AUDIT}.tgz 64ddb7e99b171a881ea2c9f4ec5efa7b988f9131949d957bcd18d17a981b5302 cargo-audit-aarch64-unknown-linux-gnu-v${AUDIT}/cargo-audit
mdbook x86_64 ${gh_release}/rust-lang/mdBook/releases/download/v${MDBOOK}/mdbook-v${MDBOOK}-x86_64-unknown-linux-gnu.tar.gz 3f28de05dafca9d0f2eab99c662116b0e37b89b1d96a08f8f430b9eeae958cd7 mdbook
mdbook aarch64 ${gh_release}/rust-lang/mdBook/releases/download/v${MDBOOK}/mdbook-v${MDBOOK}-aarch64-unknown-linux-musl.tar.gz 753e5c5c363ee8a56972344dcf91466f005a51db84a7aeffe427ae3ef83d6d44 mdbook
cargo-llvm-cov x86_64 ${gh_release}/taiki-e/cargo-llvm-cov/releases/download/v${LLVM_COV}/cargo-llvm-cov-x86_64-unknown-linux-gnu.tar.gz b3f68e625481fed9b16444174f3fa5ebcdbde4a1878803a35eabe2dcefcdc41a cargo-llvm-cov
cargo-llvm-cov aarch64 ${gh_release}/taiki-e/cargo-llvm-cov/releases/download/v${LLVM_COV}/cargo-llvm-cov-aarch64-unknown-linux-gnu.tar.gz abf5f13c1520f8756d2192bfaaeadb0208f5b7eaa8cadc15bf847befa42b7360 cargo-llvm-cov
helm x86_64 https://get.helm.sh/helm-v${HELM}-linux-amd64.tar.gz a7f81ce08007091b86d8bd696eb4d86b8d0f2e1b9f6c714be62f82f96a594496 linux-amd64/helm
helm aarch64 https://get.helm.sh/helm-v${HELM}-linux-arm64.tar.gz 440cf7add0aee27ebc93fada965523c1dc2e0ab340d4348da2215737fc0d76ad linux-arm64/helm
EOF
)"

installed=()
failed=()

# have <tool>: succeeds when the tool is already on PATH.
have() { command -v "$1" >/dev/null 2>&1; }

# from_release <tool>: downloads this architecture's archive for <tool>,
# checks its SHA-256 and installs the one binary from it.
from_release() {
  local tool="$1" url sha member archive="${tmp}/$1.archive"
  read -r url sha member < <(awk -v t="${tool}" -v a="${arch}" '$1 == t && $2 == a { print $3, $4, $5 }' <<<"${downloads}")
  [[ -n "${url:-}" ]] || { echo "no ${tool} download for ${arch}" >&2; return 1; }
  curl -sSfL --retry 3 -o "${archive}" "${url}"
  echo "${sha}  ${archive}" | sha256sum --check --quiet
  mkdir -p "${tmp}/${tool}"
  tar -xf "${archive}" -C "${tmp}/${tool}"
  install -m 0755 "${tmp}/${tool}/${member}" "${dest}/${tool}"
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

# repo_file <path>: a file from this checkout, or from develop when the
# script was piped from curl and has no checkout beside it.
repo_file() {
  local local_copy="${here}/../../$1"
  if [[ -f "${local_copy}" ]]; then
    echo "${local_copy}"
  else
    curl -sSfL --retry 3 -o "${tmp}/$(basename "$1")" "${raw}/$1"
    echo "${tmp}/$(basename "$1")"
  fi
}

for tool in vale gitleaks shellcheck lychee cargo-nextest cargo-deny cargo-audit mdbook cargo-llvm-cov; do
  want "${tool}" from_release "${tool}"
done
if [[ "${VELLUM_INSTALL_HELM:-0}" == "1" ]]; then
  want helm from_release helm
fi

# The hygiene job's Python linters (yamllint, sqlfluff, zizmor), every
# package pinned by hash as CI installs them, in a venv of their own.
hygiene() {
  local venv="${VELLUM_TOOLS_VENV:-${dest}/../share/vellum-hygiene}" requirements tool
  requirements="$(repo_file .github/requirements/hygiene.txt)"
  if have uv; then
    # CI uses Python 3.13; uv fetches it when the system has another.
    uv venv --quiet --allow-existing --python 3.13 "${venv}"
    uv pip install --quiet --python "${venv}/bin/python" --require-hashes --no-deps -r "${requirements}"
    uv pip install --quiet --python "${venv}/bin/python" "reuse==${REUSE}"
  else
    python3 -m venv "${venv}"
    "${venv}/bin/pip" install --quiet --require-hashes --no-deps -r "${requirements}"
    "${venv}/bin/pip" install --quiet "reuse==${REUSE}"
  fi
  for tool in yamllint sqlfluff zizmor reuse; do
    ln -sf "$(realpath "${venv}/bin/${tool}")" "${dest}/${tool}"
  done
}
if ! have yamllint || ! have sqlfluff || ! have zizmor || ! have reuse; then
  want yamllint hygiene
  for tool in sqlfluff zizmor reuse; do
    have "${tool}" || failed+=("${tool}")
  done
fi

want markdownlint-cli2 npm install --global --silent "markdownlint-cli2@${MARKDOWNLINT}"

# hadolint and kube-score, with the checksums CI checks.
if ! have hadolint || ! have kube-score; then
  if bash "$(repo_file scripts/ci/install-infra-tools.sh)" "${dest}" >"${tmp}/infra.log" 2>&1; then
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
