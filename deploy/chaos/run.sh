#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# The weekly chaos test (ADR 0022, D96) on a kind cluster that already runs
# the Zalando Postgres Operator and Chaos Mesh, with the images vellum:ci
# and vellum-minio:ci loaded. Under load, it kills a Vellum node, a Kroki
# pod and the Postgres primary, cuts the servers off the bucket and slows
# the database (D255), then asserts that the SLO thresholds held and that every acknowledged write
# can be read back.
#
#   deploy/chaos/run.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
K6_IMAGE="${K6_IMAGE:-grafana/k6:2.3.0@sha256:2dd4756ec6e14e62778916e1eb850108e4b499304c02df56760fa0bdb0fba0b8}"
DURATION="${DURATION:-15m}"
GAP="${GAP:-140}"
base_url="http://vellum.default.svc"
work="$(mktemp -d)"

random() { head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32; }

# k6_job NAME SCRIPT [ENV=VALUE...]: starts a k6 Job over the scripts ConfigMap.
k6_job() {
  local name="$1" script="$2"
  shift 2
  local env_args=(-e "BASE_URL=${base_url}" -e "TOKEN=${TOKEN:-}")
  for pair in "$@"; do env_args+=(-e "${pair}"); done
  kubectl create job "${name}" --image="${K6_IMAGE}" --dry-run=client -o json -- \
    k6 run -q "${env_args[@]}" "/scripts/${script}" |
    jq '.spec.backoffLimit = 0
      | .spec.template.spec.volumes = [
          {name: "scripts", configMap: {name: "k6-scripts"}},
          {name: "acks", configMap: {name: "chaos-acks", optional: true}}]
      | .spec.template.spec.containers[0].volumeMounts = [
          {name: "scripts", mountPath: "/scripts"},
          {name: "acks", mountPath: "/acks"}]' |
    kubectl apply -f -
}

# wait_job NAME TIMEOUT: succeeds when the Job completes, fails when it fails.
wait_job() {
  local name="$1" deadline=$((SECONDS + $2))
  while ((SECONDS < deadline)); do
    if [[ "$(kubectl get job "${name}" -o jsonpath='{.status.succeeded}')" == "1" ]]; then
      return 0
    fi
    if [[ "$(kubectl get job "${name}" -o jsonpath='{.status.failed}')" == "1" ]]; then
      kubectl logs "job/${name}" | tail -60
      echo "::error::${name} failed."
      return 1
    fi
    sleep 10
  done
  echo "::error::${name} did not finish in $2 s."
  return 1
}

echo "Installing MinIO and Vellum."
kubectl create secret generic vellum-kek --from-literal=passphrase="$(random)"
kubectl create secret generic minio-credentials \
  --from-literal=AWS_ACCESS_KEY_ID=vellum --from-literal=AWS_SECRET_ACCESS_KEY="$(random)"
kubectl apply -f "${here}/minio.yaml"
kubectl wait --for=condition=complete job/minio-bucket --timeout=300s
helm install vellum "${repo}/deploy/helm/vellum" -f "${here}/values-kind.yaml" \
  --set fullnameOverride=vellum --wait --timeout 15m

kubectl create configmap k6-scripts \
  --from-file="${repo}/deploy/perf/k6/" --from-file="${here}/writes.js" --from-file="${here}/verify.js"

echo "Starting load (${DURATION}) and the acknowledged-write log."
k6_job chaos-load suite.js "SCALE=0.05" "DURATION=${DURATION}" "PAGES=100"
k6_job chaos-writes writes.js "DURATION=${DURATION}"

experiments=(kill-vellum-node kill-kroki kill-postgres-primary blob-outage slow-postgres)
for experiment in "${experiments[@]}"; do
  sleep "${GAP}"
  echo "Fault: ${experiment}"
  yq "select(.metadata.name == \"${experiment}\")" "${here}/experiments.yaml" | kubectl apply -f -
done

minutes="${DURATION%m}"
wait_job chaos-writes $((minutes * 60 + 600))
kubectl logs job/chaos-writes | grep -o 'ACK [A-Za-z0-9_-]*' | cut -d' ' -f2 >"${work}/acks.txt"
echo "$(wc -l <"${work}/acks.txt") writes were acknowledged."
kubectl create configmap chaos-acks --from-file=acks.txt="${work}/acks.txt"
k6_job chaos-verify verify.js "ACKS_FILE=/acks/acks.txt"

load_ok=0
wait_job chaos-load 900 || load_ok=1
wait_job chaos-verify 900
echo "Every acknowledged write survived."
exit "${load_ok}"
