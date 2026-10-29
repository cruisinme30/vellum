#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Runs throwaway compose stacks for CI: the nightly load test, the ZAP scan
# and the release drills. Each stack has its own project name, port and
# generated secrets under $STACK_STATE (default: a temp folder).
#
#   stack.sh has-serve IMAGE                 exit 0 if IMAGE has `vellum serve`
#   stack.sh up IMAGE PROJECT PORT [KEK]     start and wait for /readyz;
#                                            REPLICAS=2 adds a second
#                                            Vellum on PORT+1
#   stack.sh up-deps PROJECT PORT [KEK]      start everything but Vellum
#   stack.sh down PROJECT                    stop and delete volumes
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compose_file="${here}/compose.yaml"
state="${STACK_STATE:-${RUNNER_TEMP:-/tmp}/vellum-stacks}"
mkdir -p "${state}"

random() { head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32; }

# compose PROJECT ARGS...: docker compose for that stack.
compose() {
  local project="$1"
  shift
  docker compose -f "${compose_file}" -p "${project}" --env-file "${state}/${project}.env" "$@"
}

# write_env PROJECT IMAGE PORT [KEK]: secrets for a new stack; an existing
# KEK file is reused so a restored stack can read the encrypted data.
write_env() {
  local project="$1" image="$2" port="$3" kek="${4:-}"
  if [[ -z "${kek}" ]]; then
    kek="${state}/${project}.kek"
    random >"${kek}"
  fi
  local csrf="${state}/${project}.csrf"
  [[ -s "${csrf}" ]] || head -c 32 /dev/urandom | base64 >"${csrf}"
  cat >"${state}/${project}.env" <<ENV
VELLUM_IMAGE=${image}
VELLUM_PORT=${port}
VELLUM_PUBLIC_URL=http://localhost:${port}
POSTGRES_PASSWORD=$(random)
MINIO_ROOT_PASSWORD=$(random)
VELLUM_CLUSTER_KEY=$(random)
VELLUM_PORT_B=$((port + 1))
VELLUM_KEK_FILE=${kek}
VELLUM_CSRF_FILE=${csrf}
VELLUM_LOG=info
OTEL_EXPORTER_OTLP_ENDPOINT=
ENV
}

wait_ready() {
  local port="$1"
  for _ in $(seq 1 120); do
    if curl -fsS "http://127.0.0.1:${port}/readyz" >/dev/null 2>&1; then
      echo "Vellum is ready on port ${port}."
      return 0
    fi
    sleep 1
  done
  echo "::error::Vellum did not become ready on port ${port} within 120 s."
  return 1
}

command="${1:-}"
shift || true
case "${command}" in
  has-serve)
    docker run --rm "$1" serve --help >/dev/null 2>&1
    ;;
  up)
    image="$1" project="$2" port="$3"
    write_env "${project}" "${image}" "${port}" "${4:-}"
    compose "${project}" up -d --wait postgres minio kroki
    compose "${project}" up -d --no-build vellum
    if ! wait_ready "${port}"; then
      compose "${project}" logs vellum | tail -100
      exit 1
    fi
    if [[ "${REPLICAS:-1}" -ge 2 ]]; then
      # Started after the first, which runs the migrations and the rescan.
      compose "${project}" --profile replicas up -d --no-build vellum-b
      if ! wait_ready "$((port + 1))"; then
        compose "${project}" --profile replicas logs vellum-b | tail -100
        exit 1
      fi
    fi
    ;;
  up-deps)
    project="$1" port="$2"
    write_env "${project}" "unused" "${port}" "${3:-}"
    compose "${project}" up -d --wait postgres minio kroki
    compose "${project}" up --no-build minio-init
    ;;
  start-vellum)
    project="$1" image="$2" port="$3"
    sed -i "s|^VELLUM_IMAGE=.*|VELLUM_IMAGE=${image}|" "${state}/${project}.env"
    compose "${project}" up -d --no-build --force-recreate vellum
    wait_ready "${port}"
    ;;
  compose)
    compose "$@"
    ;;
  down)
    compose "$1" --profile replicas down --volumes --remove-orphans
    ;;
  *)
    echo "usage: stack.sh has-serve IMAGE | up IMAGE PROJECT PORT [KEK] | up-deps PROJECT PORT [KEK]" >&2
    echo "       | start-vellum PROJECT IMAGE PORT | compose PROJECT ARGS... | down PROJECT" >&2
    exit 2
    ;;
esac
