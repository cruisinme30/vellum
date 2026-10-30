#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Installs the pinned infrastructure linters (hadolint, kube-score) into
# $1 (default: $RUNNER_TEMP/bin) after checking their SHA-256 sums. Helm
# comes from azure/setup-helm in the workflow.
set -euo pipefail

dest="${1:-${RUNNER_TEMP:?}/bin}"
mkdir -p "${dest}"
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

# fetch <url> <sha256> <output>
fetch() {
  curl -sSfL --retry 3 -o "$3" "$1"
  echo "$2  $3" | sha256sum --check --quiet
}

# The release assets and their SHA-256 sums, per architecture.
case "$(uname -m)" in
  x86_64)
    hadolint=(hadolint-linux-x86_64 6bf226944684f56c84dd014e8b979d27425c0148f61b3bd99bcc6f39e9dc5a47)
    kube_score=(kube-score_1.20.0_linux_amd64.tar.gz ff777b4b3f45905b2d22c69e93c57a7a170e61529a79d54e59cb95cca4bc2a55)
    ;;
  aarch64 | arm64)
    hadolint=(hadolint-linux-arm64 331f1d3511b84a4f1e3d18d52fec284723e4019552f4f47b19322a53ce9a40ed)
    kube_score=(kube-score_1.20.0_linux_arm64.tar.gz 0fafe8a00cbc98935f0c0953ba91e83e199f65e56d6816e7b2d2af5da1862051)
    ;;
  *)
    echo "install-infra-tools.sh: no hadolint or kube-score for $(uname -m)" >&2
    exit 1
    ;;
esac

fetch "https://github.com/hadolint/hadolint/releases/download/v2.14.0/${hadolint[0]}" \
  "${hadolint[1]}" "${tmp}/hadolint"
install -m 0755 "${tmp}/hadolint" "${dest}/hadolint"

fetch "https://github.com/zegl/kube-score/releases/download/v1.20.0/${kube_score[0]}" \
  "${kube_score[1]}" "${tmp}/kube-score.tar.gz"
tar -xzf "${tmp}/kube-score.tar.gz" -C "${tmp}" kube-score
install -m 0755 "${tmp}/kube-score" "${dest}/kube-score"

if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "${dest}" >>"${GITHUB_PATH}"
fi
echo "Installed hadolint and kube-score into ${dest}."
