#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# The local CI gate (ADR 0073, D318): runs CI's own checks on the committed
# HEAD tree, in Linux containers, before anything reaches GitHub. On
# success it stamps the tree, and the pre-push hook lets a push to develop,
# main or chore/release-* through only with that stamp.
#
#   scripts/dev/gate.sh             what ci.yml runs: the Fast suite, the
#                                   website and the UI suite, here the whole
#                                   suite in Chromium, Firefox and WebKit
#   scripts/dev/gate.sh --release   also what full.yml and codeql.yml run on
#                                   a release pull request
#
# Stages, each with its own log, side by side where memory allows:
#   image      builds vellum-ci-local (scripts/dev/ci-image/Dockerfile) when
#              its pinned inputs changed
#   prepare    checks HEAD out into a Docker volume: the gate tests exactly
#              the commit, never uncommitted edits, and writes nothing into
#              the checkout but the stamp
#   fast       `cargo xtask fast` with every area and every test, the
#              Postgres, Meilisearch and S3 tests included (containers on
#              the host's Docker, whose published ports a container reaches
#              at host.docker.internal)
#   site       scripts/ci/build-site.sh and check-site.sh
#   e2e-*      the whole Playwright suite (UI_SCOPE=full) per browser, in the
#              pinned Playwright image, with visual baselines in Chromium; it
#              contains the Fast suite's quick UI layout run
# With --release (full.yml and codeql.yml):
#   coverage   the Rust tests under cargo llvm-cov, at least 85 % of lines,
#              then the xtask tests
#   x86        two from-scratch amd64 release builds that must be
#              byte-identical, the runtime image from them, then `cargo
#              xtask budgets`, all under --platform linux/amd64
#   clamav     ClamAV from the Compose stack takes a 30 MB upload and EICAR
#   links      lychee on every Markdown file, external links included
#   reliability  the latest nightly load, ZAP, fuzz and chaos runs passed
#              (scripts/ci/reliability-gate.sh, with your gh login)
#   codeql     CodeQL security-extended for actions and JavaScript, with the
#              CLI from $VELLUM_CODEQL
#
# What it cannot check is printed at the end of every run, never hidden.
#
# Options:
#   --release            add the release stages
#   --only <stages>      run only these (comma-separated stage names; e2e
#                        means every browser); writes no stamp. The pre-push
#                        hook uses --only fast on feature branches
#   --focused            with --only fast: check only what the branch
#                        changed, as `cargo xtask fast` does by default
#   --workers <n>        Playwright workers per browser (default 2)
#   --e2e-parallel <n>   browsers at once (default from Docker's memory)
#   --jobs <n>           cargo build jobs (default 4)
#   --browsers <list>    only these browsers (comma-separated); no stamp
#   --grep <pattern>     only the UI tests matching it (Playwright -g); no
#                        stamp. For a quick look at one failing test
#   -h, --help           this text
#
# Caches live in named Docker volumes (vellum-gate-*); remove them with
# `docker volume ls -q --filter name=vellum-gate- | xargs docker volume rm`.
# Every container carries the label vellum-gate=<run id> and is removed
# on exit, failure or Ctrl-C.
set -euo pipefail
# One block, so bash reads the whole file before running any of it: a
# checkout that changes this file mid-run cannot change what runs.
# shellcheck disable=SC2329 # stages are called through run_stage
{

if ((BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] < 501)); then
  echo "gate.sh needs bash 5.1 or later (on a Mac: brew install bash)." >&2
  exit 2
fi

usage() { sed -n '/^# The local CI gate/,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'; }

release=0
focused=0
only=""
workers=2
parallel=""
jobs=4
browsers=chromium,firefox,webkit
grep_tests=""
while (($# > 0)); do
  case "$1" in
    --release) release=1 ;;
    --only) only="${2:?--only needs a list of stages}"; shift ;;
    --focused) focused=1 ;;
    --workers) workers="${2:?}"; shift ;;
    --e2e-parallel) parallel="${2:?}"; shift ;;
    --jobs) jobs="${2:?}"; shift ;;
    --browsers) browsers="${2:?}"; shift ;;
    --grep) grep_tests="${2:?}"; shift ;;
    -h | --help) usage; exit 0 ;;
    *) echo "gate.sh: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

# The stages to run.
if ((focused)) && [[ "${only}" != fast ]]; then
  echo "gate.sh: --focused goes with --only fast." >&2
  exit 2
fi
readonly RELEASE_STAGES="coverage,x86,clamav,links,reliability,codeql"
if [[ -n "${only}" ]]; then
  wanted=",${only},"
else
  wanted=",fast,site,e2e,"
  ((release)) && wanted+="${RELEASE_STAGES},"
fi
want() { [[ "${wanted}" == *",$1,"* ]]; }
# The UI suite needs the website.
want e2e && wanted+="site,"

readonly PLAYWRIGHT_IMAGE=mcr.microsoft.com/playwright:v1.64.1-noble
IFS=, read -r -a BROWSERS <<<"${browsers}"
readonly BROWSERS
# A narrowed run checks less than CI does, so it earns no stamp.
partial="${only}"
[[ "${browsers}" != chromium,firefox,webkit ]] && partial+=" --browsers ${browsers}"
[[ -n "${grep_tests}" ]] && partial+=" --grep"

root="$(git rev-parse --show-toplevel)"
cd "${root}"
common="$(cd "$(git rev-parse --git-common-dir)" && pwd -P)"
if ! docker info >/dev/null 2>&1; then
  echo "gate.sh: Docker is not running." >&2
  exit 2
fi
# Gate runs share their volumes, so one at a time. Other tools may use the
# label too; the gate's own containers are also named vellum-gate-<run>-*.
if [[ -n "$(docker ps -q --filter label=vellum-gate --filter name=^vellum-gate-)" ]]; then
  echo "gate.sh: another gate run is still going (docker ps --filter name=^vellum-gate-)." >&2
  exit 2
fi

git fetch --quiet origin develop 2>/dev/null || true
sha="$(git rev-parse HEAD)"
tree="$(git rev-parse 'HEAD^{tree}')"
branch="$(git symbolic-ref --quiet --short HEAD || true)"
base="$(git rev-parse --quiet --verify origin/develop || true)"
repo_slug="$(git remote get-url origin 2>/dev/null | sed -E 's#(\.git)?$##; s#^.*github\.com[:/]##')"
run_id="$(date +%Y%m%d-%H%M%S)-$$"
compose_project="vellum-gate-${run_id}"
logs="$(mktemp -d "${TMPDIR:-/tmp}/vellum-gate.XXXXXX")"
logs="$(cd "${logs}" && pwd -P)"
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  echo "Note: uncommitted changes are not checked; the gate tests the commit ${sha:0:12}."
fi

memory_gb=$(($(docker info --format '{{.MemTotal}}') / 1024 / 1024 / 1024))
if [[ -z "${parallel}" ]]; then
  # About 3 GB per browser beside the Rust build.
  if ((memory_gb >= 16)); then parallel=3; elif ((memory_gb >= 11)); then parallel=2; else parallel=1; fi
fi

# --- cleanup -----------------------------------------------------------------

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
before_run="$(docker ps -aq | tr '\n' ' ')"
interrupted=0
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  local -a pids leftovers
  mapfile -t pids < <(jobs -pr)
  ((${#pids[@]} == 0)) || kill "${pids[@]}" 2>/dev/null || true
  mapfile -t leftovers < <(docker ps -aq --filter "label=vellum-gate=${run_id}"
    docker ps -aq --filter "label=com.docker.compose.project=${compose_project}")
  ((${#leftovers[@]} == 0)) || docker rm -f -v "${leftovers[@]}" >/dev/null 2>&1 || true
  mapfile -t leftovers < <(docker network ls -q --filter "label=vellum-gate=${run_id}"
    docker network ls -q --filter "label=com.docker.compose.project=${compose_project}")
  ((${#leftovers[@]} == 0)) || docker network rm "${leftovers[@]}" >/dev/null 2>&1 || true
  mapfile -t leftovers < <(docker volume ls -q --filter "label=com.docker.compose.project=${compose_project}")
  ((${#leftovers[@]} == 0)) || docker volume rm "${leftovers[@]}" >/dev/null 2>&1 || true
  docker image rm -f "vellum-gate-release:${run_id}" >/dev/null 2>&1 || true
  wait 2>/dev/null || true
  if ((interrupted)); then
    echo
    echo "Interrupted; removed this run's containers. Logs: ${logs}"
  fi
  # Tests start Postgres, Meilisearch and LocalStack containers themselves
  # and remove them when they end; a test that is killed or times out
  # cannot. Those are listed, not removed: they carry no gate label, and
  # other work on this Docker may own containers started meanwhile.
  local id
  for id in $(docker ps -aq --filter label=org.testcontainers=true; docker ps -aq --filter ancestor=localstack/localstack:4); do
    [[ " ${before_run} " == *" ${id} "* ]] && continue
    docker ps -a --filter "id=${id}" --format 'Started during this run and still there (a killed test?): {{.ID}} {{.Image}} {{.Status}}'
  done
  exit "${status}"
}
trap cleanup EXIT
trap 'interrupted=1; exit 130' INT TERM

# --- stages ------------------------------------------------------------------

stages=()
# run_stage <name> <command...>: runs a stage into its own log and records
# its exit status and duration.
run_stage() {
  local name="$1" start="${SECONDS}" status=0
  shift
  echo "  start  ${name}"
  "$@" >"${logs}/${name}.log" 2>&1 || status=$?
  echo "${status} $((SECONDS - start))" >"${logs}/${name}.status"
  if ((status == 0)); then
    echo "  pass   ${name} ($(duration $((SECONDS - start))))"
  else
    echo "  FAIL   ${name} ($(duration $((SECONDS - start)))): ${logs}/${name}.log"
  fi
  return "${status}"
}
skip_stage() { echo "skip 0 $2" >"${logs}/$1.status"; }
duration() { printf '%dm%02ds' $(($1 / 60)) $(($1 % 60)); }
passed() { [[ -f "${logs}/$1.status" ]] && [[ "$(cut -d' ' -f1 "${logs}/$1.status")" == 0 ]]; }

# drun <stage> <docker run args...>: a labelled, self-removing container.
drun() {
  local name="$1"
  shift
  docker run --rm --init --label "vellum-gate=${run_id}" --name "vellum-gate-${run_id}-${name}" "$@"
}

# The image's inputs: the files that pin its tools.
image_inputs=(scripts/dev/ci-image/Dockerfile rust-toolchain.toml .github/requirements/hygiene.txt
  scripts/dev/install-tools.sh scripts/ci/install-infra-tools.sh web/package.json)
image_tag="$(git ls-tree HEAD -- "${image_inputs[@]}" | git hash-object --stdin | cut -c1-12)"
ci_image="vellum-ci-local:${image_tag}"

# build_image <platform> <tag>: builds vellum-ci-local from HEAD's pins.
build_image() {
  if docker image inspect "$2" >/dev/null 2>&1; then
    echo "$2 is up to date."
    return 0
  fi
  git archive HEAD "${image_inputs[@]}" |
    docker build --platform "$1" -f scripts/dev/ci-image/Dockerfile -t "$2" -
}

stage_image() {
  build_image linux/arm64 "${ci_image}"
  docker tag "${ci_image}" vellum-ci-local:latest
}

# zizmor's online audits (impostor commits, known-vulnerable actions) need
# a GitHub token, as in CI. The gate passes GH_TOKEN only when you set it.
token_env=()
[[ -n "${GH_TOKEN:-}" ]] && token_env=(-e GH_TOKEN)

# Mounts every vellum-ci-local container shares: the checkout volume, the
# repository's object store (read-only, at its own path, which the
# checkout borrows objects from) and the cargo caches.
ci_mounts=(
  -v vellum-gate-work:/work
  -v "${common}:${common}:ro"
  -v vellum-gate-cargo-registry:/usr/local/cargo/registry
  -v vellum-gate-cargo-git:/usr/local/cargo/git
  -v vellum-gate-pnpm-store:/cache/pnpm-store
  -e npm_config_store_dir=/cache/pnpm-store
  -e CI=true
  -e "CARGO_BUILD_JOBS=${jobs}"
  -e "RUSTFLAGS=-D warnings -C link-arg=-fuse-ld=lld"
  -e "RUSTDOCFLAGS=-D warnings"
  -e CARGO_TERM_COLOR=never
)
# Containers whose tests start containers of their own.
docker_mounts=(
  --network host
  -v /var/run/docker.sock:/var/run/docker.sock
  -e TESTCONTAINERS_HOST_OVERRIDE=host.docker.internal
)

# The commit, checked out into the vellum-gate-work volume at /work/vellum
# and archived to /work/src.tar for the browser stages. The checkout shares
# objects with the repository, so it takes seconds.
stage_prepare() {
  # shellcheck disable=SC2016 # expanded in the container
  drun prepare --platform linux/arm64 --pull never "${ci_mounts[@]}" \
    -e "COMMON=${common}" -e "SHA=${sha}" -e "BRANCH=${branch}" -e "BASE=${base}" \
    "${ci_image}" bash -euo pipefail -c '
      src=/work/vellum
      if ! git -C "${src}" rev-parse --git-dir >/dev/null 2>&1; then
        rm -rf "${src}"
        git clone --quiet --shared --no-checkout "${COMMON}" "${src}"
      fi
      cd "${src}"
      if [[ -n "${BRANCH}" ]]; then
        git checkout --quiet --force -B "${BRANCH}" "${SHA}"
      else
        git checkout --quiet --force --detach "${SHA}"
      fi
      git clean -ffdxq -e /target -e /web/node_modules -e /desktop/node_modules
      if [[ -n "${BASE}" ]]; then
        git update-ref refs/remotes/origin/develop "${BASE}"
      else
        git update-ref -d refs/remotes/origin/develop 2>/dev/null || true
      fi
      git archive --format=tar -o /work/src.tar HEAD
      rm -rf /work/_site
      echo "Checked out $(git rev-parse --short HEAD) (${BRANCH:-detached}), base ${BASE:0:12}."
    '
}

# The full UI suite (the e2e stages) contains the quick UI layout run, so
# when they run, the Fast suite leaves that one to them.
stage_fast() {
  local scope=(-e VELLUM_FAST_ALL_TESTS=1 -e VELLUM_FAST_ALL_AREAS=1)
  ((focused)) && scope=()
  want e2e && scope+=(-e VELLUM_FAST_SKIP_UI=1)
  drun fast --platform linux/arm64 --pull never "${docker_mounts[@]}" \
    -v vellum-gate-target:/work/vellum/target \
    "${ci_mounts[@]}" "${token_env[@]}" -w /work/vellum "${scope[@]}" \
    -e ELECTRON_SKIP_BINARY_DOWNLOAD=1 \
    "${ci_image}" cargo xtask fast
}

# CI's website job: no tags in its shallow checkout, so no older versions.
stage_site() {
  drun site --platform linux/arm64 --pull never \
    -v vellum-gate-target-site:/cache/target \
    "${ci_mounts[@]}" -w /work/vellum \
    -e CARGO_TARGET_DIR=/cache/target -e SITE_VERSIONS=0 \
    "${ci_image}" bash -euo pipefail -c \
    'scripts/ci/build-site.sh /work/_site && scripts/ci/check-site.sh /work/_site'
}

# The Playwright containers unpack the commit into /e2e/vellum and keep
# web/node_modules in a volume of their own, installed once by pw-install.
pw_mounts=(
  -v vellum-gate-work:/work:ro
  -v vellum-gate-pw-node-modules:/e2e/vellum/web/node_modules
  -v vellum-gate-pnpm-store:/cache/pnpm-store
  -v vellum-gate-corepack:/cache/corepack
  -e npm_config_store_dir=/cache/pnpm-store
  -e COREPACK_HOME=/cache/corepack
  -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0
  -e CI=true
  -e PATH=/cache/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
)
# pnpm through corepack, at web/package.json's packageManager version.
readonly PW_SETUP='mkdir -p /cache/bin && corepack enable --install-directory /cache/bin pnpm &&
  tar -xf /work/src.tar -C /e2e/vellum && cd /e2e/vellum/web'

stage_pw_install() {
  drun pw-install --platform linux/arm64 "${pw_mounts[@]}" "${PLAYWRIGHT_IMAGE}" \
    bash -euo pipefail -c "${PW_SETUP} && pnpm install --frozen-lockfile"
}

# stage_e2e <browser>: full.yml's end-to-end job for one browser, every
# shard in one container. A failure keeps the report beside the log.
stage_e2e() {
  local browser="$1" visual=0 out="${logs}/e2e-$1"
  [[ "${browser}" == chromium ]] && visual=1
  mkdir -p "${out}"
  # shellcheck disable=SC2016 # expanded in the container
  drun "e2e-${browser}" --platform linux/arm64 --ipc=host "${pw_mounts[@]}" \
    -v "${out}:/out" \
    -e UI_SCOPE=full -e "UI_BROWSERS=${browser}" -e UI_SITE_DIR=/work/_site \
    -e "UI_VISUAL=${visual}" -e "WORKERS=${workers}" -e "GREP=${grep_tests}" \
    "${PLAYWRIGHT_IMAGE}" bash -uo pipefail -c "${PW_SETUP}"' || exit 1
      pnpm exec playwright test --workers="${WORKERS}" ${GREP:+-g "${GREP}"}
      status=$?
      if ((status != 0)); then cp -R playwright-report test-results /out/ 2>/dev/null; fi
      exit "${status}"'
}

# The browsers, ${parallel} at a time. Waits only on its own jobs: the
# fast stage runs in the background meanwhile.
run_e2e() {
  local browser done_pid
  local -a running=()
  for browser in "${BROWSERS[@]}"; do
    while ((${#running[@]} >= parallel)); do
      wait -n -p done_pid "${running[@]}" || true
      running=("${running[@]/#${done_pid}/}")
      mapfile -t running < <(printf '%s\n' "${running[@]}" | grep . || true)
    done
    run_stage "e2e-${browser}" stage_e2e "${browser}" &
    running+=($!)
  done
  ((${#running[@]} == 0)) || wait "${running[@]}" || true
}

# full.yml's coverage job, the same two commands.
stage_coverage() {
  drun coverage --platform linux/arm64 --pull never "${docker_mounts[@]}" \
    -v vellum-gate-target-cov:/work/vellum/target \
    "${ci_mounts[@]}" -w /work/vellum -e NEXTEST_PROFILE=ci \
    "${ci_image}" bash -euo pipefail -c '
      cargo llvm-cov nextest --workspace --exclude xtask --locked \
        --fail-under-lines 85 --lcov --output-path /tmp/lcov.info
      cargo nextest run --package xtask --locked --profile ci'
}

# full.yml's binary and image jobs: two from-scratch amd64 builds of the
# release binary, which must be byte-identical, the runtime image from the
# first, then the budgets, run by an amd64 vellum-ci-local.
stage_x86() {
  local out="${logs}/x86" image="vellum-gate-release:${run_id}" bytes build
  mkdir -p "${out}/dist"
  build_image linux/amd64 "${ci_image}-amd64"
  for build in 1 2; do
    git archive HEAD | docker buildx build --no-cache --platform linux/amd64 --target binary \
      --output "type=local,dest=${out}/build-${build}" -f deploy/docker/Dockerfile -
  done
  shasum -a 256 "${out}"/build-*/vellum
  cmp "${out}/build-1/vellum" "${out}/build-2/vellum"
  cp "${out}/build-1/vellum" "${out}/dist/vellum-linux-amd64"
  git archive HEAD | docker buildx build --platform linux/amd64 --load --target runtime \
    --build-arg DIST=release --build-context "release=${out}/dist" \
    -t "${image}" -f deploy/docker/Dockerfile -
  bytes="$(docker save "${image}" | gzip -9 | wc -c | tr -d ' ')"
  echo "Compressed image: ${bytes} bytes"
  drun x86-budgets --platform linux/amd64 --pull never \
    -v vellum-gate-target-amd64:/work/vellum/target \
    "${ci_mounts[@]}" -v "${out}:/out" -w /work/vellum \
    "${ci_image}-amd64" cargo run --locked --package xtask -- budgets \
    --binary /out/build-1/vellum --image-bytes "${bytes}"
}

# full.yml's clamav job: ClamAV from the Compose stack, then smoke.sh from
# a container on the stack's network. The stack runs from an export of the
# commit, so the .env it needs is not written into the checkout.
stage_clamav() {
  local dir="${logs}/clamav" ip status=0
  mkdir -p "${dir}"
  git archive HEAD deploy | tar -x -C "${dir}"
  cp "${dir}/deploy/compose/.env.example" "${dir}/deploy/compose/.env"
  local compose=(docker compose -p "${compose_project}" --project-directory "${dir}/deploy/compose"
    -f "${dir}/deploy/compose/compose.yaml")
  "${compose[@]}" up -d --wait --wait-timeout 900 clamav || status=$?
  if ((status == 0)); then
    ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' \
      "$("${compose[@]}" ps -q clamav)")"
    drun clamav-smoke --platform linux/arm64 --pull never \
      --network "$(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}' \
        "$("${compose[@]}" ps -q clamav)")" \
      -v "${dir}/deploy:/deploy:ro" "${ci_image}" /deploy/clamav/smoke.sh "${ip}" 3310 || status=$?
  fi
  ((status == 0)) || "${compose[@]}" logs clamav
  "${compose[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  return "${status}"
}

# full.yml's links job: lychee online, with the same arguments.
stage_links() {
  drun links --platform linux/arm64 --pull never "${ci_mounts[@]}" "${token_env[@]}" \
    -w /work/vellum "${ci_image}" lychee --no-progress --exclude-path LICENSES \
    --exclude-path web/node_modules --exclude-path corpus '**/*.md'
}

# full.yml's reliability job, on this machine with your gh login.
stage_reliability() {
  if ! command -v gh >/dev/null || ! command -v jq >/dev/null; then
    echo "gh and jq are needed to read the nightly results."
    return 1
  fi
  GH_REPO="${repo_slug}" scripts/ci/reliability-gate.sh
}

# codeql.yml: security-extended, build mode none, for the languages the
# macOS CLI can extract. Fails on error-level results, which code
# scanning's check reports as failures.
stage_codeql() {
  local codeql="${VELLUM_CODEQL:-}" dir="${logs}/codeql" lang pack errors total
  if [[ -z "${codeql}" ]]; then
    codeql="$(command -v codeql || echo "$(dirname "$(dirname "${common}")")/vellum-tools/codeql/codeql")"
  fi
  if [[ ! -x "${codeql}" ]]; then
    echo "No CodeQL CLI: set VELLUM_CODEQL to the codeql binary of a CodeQL bundle."
    return 1
  fi
  mkdir -p "${dir}/src"
  git archive HEAD | tar -x -C "${dir}/src"
  cat >"${dir}/config.yml" <<'EOF'
paths-ignore:
  - web/dist
  - web/coverage
  - "**/node_modules"
EOF
  for lang in actions javascript-typescript; do
    pack="${lang%%-*}"
    "${codeql}" database create "${dir}/db-${lang}" --language="${lang}" --build-mode=none \
      --source-root="${dir}/src" --codescanning-config="${dir}/config.yml" --threads=2 --overwrite
    "${codeql}" database analyze "${dir}/db-${lang}" \
      "codeql/${pack}-queries:codeql-suites/${pack}-security-extended.qls" \
      --format=sarif-latest --output="${dir}/${lang}.sarif" --threads=2
  done
  read -r errors total < <(python3 -I - "${dir}"/*.sarif <<'EOF'
import json, sys
errors = total = 0
for path in sys.argv[1:]:
    for run in json.load(open(path))["runs"]:
        for result in run.get("results", []):
            total += 1
            errors += result.get("level", "warning") == "error"
            print(f"{result.get('level', 'warning')}: {result['ruleId']}: "
                  f"{result['locations'][0]['physicalLocation']['artifactLocation']['uri']}",
                  file=sys.stderr)
print(errors, total)
EOF
  )
  echo "CodeQL (actions, JavaScript): ${total} result(s), ${errors} at error level."
  ((errors == 0))
}

# --- run ---------------------------------------------------------------------

echo "Vellum gate ${run_id}: ${sha:0:12} (tree ${tree:0:12}) on ${branch:-a detached HEAD}"
echo "Logs: ${logs}   Docker memory: ${memory_gb} GB, browsers at once: ${parallel}"
start="${SECONDS}"

run_stage image stage_image
run_stage prepare stage_prepare
stages+=(image prepare)

# Light release stages run beside everything else; the heavy ones (a
# second workspace build, emulated amd64 builds, ClamAV's signatures) wait
# for the main stages unless Docker has room to spare.
background=()
for name in links reliability codeql; do
  if want "${name}"; then run_stage "${name}" "stage_${name}" & background+=($!); stages+=("${name}"); fi
done
heavy=()
for name in coverage x86 clamav; do want "${name}" && heavy+=("${name}"); done
if ((memory_gb >= 16)); then
  for name in "${heavy[@]}"; do run_stage "${name}" "stage_${name}" & background+=($!); done
fi

if want fast; then run_stage fast stage_fast & background+=($!); stages+=(fast); fi
if want site; then run_stage site stage_site & site_pid=$!; stages+=(site); fi
if want e2e; then run_stage pw-install stage_pw_install & pw_pid=$!; stages+=(pw-install); fi

if want e2e; then
  wait "${site_pid}" || true
  wait "${pw_pid}" || true
  if passed site && passed pw-install; then
    run_e2e
  else
    for browser in "${BROWSERS[@]}"; do skip_stage "e2e-${browser}" "website or install failed"; done
  fi
  for browser in "${BROWSERS[@]}"; do stages+=("e2e-${browser}"); done
elif want site; then
  wait "${site_pid}" || true
fi
for pid in "${background[@]}"; do wait "${pid}" || true; done
if ((memory_gb < 16)); then
  for name in "${heavy[@]}"; do run_stage "${name}" "stage_${name}" || true; done
fi
stages+=("${heavy[@]}")

# --- report ------------------------------------------------------------------

failed=0
echo
printf '%-12s %-6s %8s  %s\n' Stage Result Time Log
for name in "${stages[@]}"; do
  read -r status seconds rest <"${logs}/${name}.status" || { status=1; seconds=0; }
  if [[ "${status}" == skip ]]; then
    result=NOT-RUN
    failed=1
  elif [[ "${status}" == 0 ]]; then
    result=pass
  else
    result=FAIL
    failed=1
  fi
  printf '%-12s %-6s %8s  %s\n' "${name}" "${result}" "$(duration "${seconds}")" \
    "${rest:-${logs}/${name}.log}"
done
echo "Total: $(duration $((SECONDS - start)))"

# Every CI check this run did not perform, so no one mistakes a stamp for
# more than it is.
echo
echo "NOT CHECKED BY THIS RUN (CI still runs these):"
echo "  - pull request title, description, linked issue and decision record:"
echo "    run cargo xtask pr-check before opening the pull request"
echo "  - dependency review of the pull request (GitHub only)"
[[ -z "${GH_TOKEN:-}" ]] && want fast &&
  echo "  - zizmor's online audits: no GH_TOKEN set, so only its offline audits ran"
if ((release)) || want codeql; then
  echo "  - CodeQL for Rust: the macOS CodeQL CLI cannot extract Rust code"
else
  echo "  - CodeQL, coverage, release builds, budgets, ClamAV, external links and"
  echo "    nightly results: run with --release (main and release branches need it)"
fi
if want e2e; then
  echo "  - CI runs the UI suite in four shards of four workers per browser; here"
  echo "    each browser runs whole, with ${workers} workers (same tests)"
fi
echo

if ((failed)); then
  echo "The gate failed; fix the stages marked FAIL. Logs: ${logs}"
  exit 1
fi
if [[ -n "${partial}" ]]; then
  echo "Passed (partial run: ${partial# }; no stamp written)."
  exit 0
fi

# The stamp names the tree, so rewording a commit keeps it. A release stamp
# also satisfies the develop check; a later standard run keeps it.
mode=standard
((release)) && mode=release
stamp_dir="${common}/vellum-gate"
mkdir -p "${stamp_dir}"
if ! grep -qx 'mode=release' "${stamp_dir}/${tree}" 2>/dev/null || ((release)); then
  printf 'mode=%s\ncommit=%s\nbranch=%s\nstarted=%s\nrun=%s\n' \
    "${mode}" "${sha}" "${branch}" "${started_at}" "${run_id}" >"${stamp_dir}/${tree}"
fi
echo "Passed. Stamped tree ${tree:0:12} (${mode}); pushes of this tree may go ahead."
exit 0
}
