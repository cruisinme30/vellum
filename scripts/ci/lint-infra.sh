#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Lints SQL, Dockerfiles, Compose files, Helm charts and Kubernetes
# manifests. Each linter runs as soon as its first file exists; a missing
# tool then fails CI. Outside CI (the pre-push hook) a missing tool is
# reported and skipped, so contributors need not install every linter.
set -euo pipefail

failures=0

# missing <tool> <what>: fails in CI, warns locally.
missing() {
  if [[ "${CI:-}" == "true" ]]; then
    echo "::error::$2 exist but $1 is not installed in CI."
    failures=$((failures + 1))
  else
    echo "$2: $1 is not installed; skipped here, CI runs it."
  fi
}

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
    missing "${tool}" "${what}"
    return
  fi
  echo "${what}: ${tool} on ${#files[@]} file(s)."
  "$@" "${files[@]}" || failures=$((failures + 1))
}

mapfile -t sql < <(git ls-files '*.sql')
mapfile -t dockerfiles < <(git ls-files '*Dockerfile*')
mapfile -t charts < <(git ls-files '*Chart.yaml' | xargs -r -n1 dirname)
mapfile -t manifests < <(git ls-files 'deploy/k8s/*.yaml')
mapfile -t composes < <(git ls-files '*compose.yaml')

run_if_files sqlfluff "SQL" "${sql[@]}" -- sqlfluff lint --dialect postgres
run_if_files hadolint "Dockerfiles" "${dockerfiles[@]}" -- hadolint
run_if_files helm "Helm charts" "${charts[@]}" -- helm lint --strict
run_if_files kube-score "Kubernetes manifests" "${manifests[@]}" \
  -- kube-score score

# Compose files must resolve with their example environment.
for file in "${composes[@]}"; do
  if ! docker compose version >/dev/null 2>&1; then
    missing "docker compose" "Compose files"
    break
  fi
  echo "Compose: ${file}"
  docker compose -f "${file}" --env-file "$(dirname "${file}")/.env.example" \
    config -q ||
    failures=$((failures + 1))
done

# Each chart renders with every ci/*-values.yaml file; kube-score checks the
# rendered objects. It cannot read Argo Rollouts kinds, so canary renderings
# are linted only.
for chart in "${charts[@]}"; do
  if ! command -v helm >/dev/null || ! command -v kube-score >/dev/null; then
    missing "helm or kube-score" "Rendered Helm charts"
    break
  fi
  for values in "${chart}"/ci/*-values.yaml; do
    [[ -e "${values}" ]] || continue
    echo "Helm chart ${chart} with ${values}"
    helm lint --strict "${chart}" -f "${values}" || failures=$((failures + 1))
    if [[ "${values}" != *canary* ]]; then
      helm template ci "${chart}" -f "${values}" | kube-score score - ||
        failures=$((failures + 1))
    fi
  done
done

exit $((failures > 0))
