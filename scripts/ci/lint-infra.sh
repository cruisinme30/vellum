#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Lints SQL, Dockerfiles, Helm charts and Kubernetes manifests. Each linter
# runs as soon as its first file exists; a missing tool then fails CI.
set -euo pipefail

failures=0

# run_if_files <tool> <description> <files...> -- <command...>
run_if_files() {
  local tool="$1" what="$2"
  shift 2
  local files=()
  while (($# > 0)) && [[ "$1" != "--" ]]; do
    files+=("$1")
    shift
  done
  shift
  if ((${#files[@]} == 0)); then
    echo "${what}: no files yet, nothing to lint."
    return
  fi
  if ! command -v "${tool}" >/dev/null; then
    echo "::error::${what} exist but ${tool} is not installed in CI."
    failures=$((failures + 1))
    return
  fi
  echo "${what}: ${tool} on ${#files[@]} file(s)."
  "$@" "${files[@]}" || failures=$((failures + 1))
}

mapfile -t sql < <(git ls-files '*.sql')
mapfile -t dockerfiles < <(git ls-files '*Dockerfile*')
mapfile -t charts < <(git ls-files '*Chart.yaml' | xargs -r -n1 dirname)
mapfile -t manifests < <(git ls-files 'deploy/k8s/*.yaml')

run_if_files sqlfluff "SQL" "${sql[@]}" -- sqlfluff lint --dialect postgres
run_if_files hadolint "Dockerfiles" "${dockerfiles[@]}" -- hadolint
run_if_files helm "Helm charts" "${charts[@]}" -- helm lint --strict
run_if_files kube-score "Kubernetes manifests" "${manifests[@]}" -- kube-score score

exit $((failures > 0))
