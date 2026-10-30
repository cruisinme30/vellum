#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# The one way to push (ADR 0073, D318): pushes a commit that passed the
# local gate (scripts/dev/gate.sh) to a branch on origin.
#
#   scripts/dev/push.sh <branch> [<commit>] [--supersede] [--dry-run]
#
#   <branch>      the branch on origin, such as develop or feat/typed-ids
#   <commit>      what to push (default HEAD)
#   --supersede   push to develop even while a Fast suite run on develop is
#                 in progress (the new push's run follows it)
#   --dry-run     make every check, then print the push instead of running it
#
# It refuses unless the gate stamped the commit's tree; main and
# chore/release-* need a `gate.sh --release` stamp. For develop it also
# refuses while a Fast suite run on develop is in progress, unless
# --supersede is given. It then runs
#   git push origin <commit>:refs/heads/<branch>
# and, for chore/release-*, adds --force-with-lease against the branch's
# current SHA on origin, so a release branch is replaced only if no one
# else moved it meanwhile.
set -euo pipefail

usage() {
  sed -n '/^# The one way to push/,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
}

branch=""
commit=""
supersede=0
dry_run=0
while (($# > 0)); do
  case "$1" in
    --supersede) supersede=1 ;;
    --dry-run) dry_run=1 ;;
    -h | --help) usage; exit 0 ;;
    -*) echo "push.sh: unknown option $1" >&2; usage >&2; exit 2 ;;
    *)
      if [[ -z "${branch}" ]]; then
        branch="$1"
      elif [[ -z "${commit}" ]]; then
        commit="$1"
      else
        echo "push.sh: too many arguments" >&2
        usage >&2
        exit 2
      fi
      ;;
  esac
  shift
done
if [[ -z "${branch}" ]]; then
  usage >&2
  exit 2
fi
if ! git check-ref-format --branch "${branch}" >/dev/null 2>&1; then
  echo "push.sh: '${branch}' is not a valid branch name." >&2
  exit 2
fi

cd "$(git rev-parse --show-toplevel)"
sha="$(git rev-parse --verify --quiet "${commit:-HEAD}^{commit}")" || {
  echo "push.sh: '${commit:-HEAD}' is not a commit." >&2
  exit 2
}
tree="$(git rev-parse "${sha}^{tree}")"
stamp="$(git rev-parse --git-common-dir)/vellum-gate/${tree}"

need=standard
case "${branch}" in
  main | chore/release-*) need=release ;;
esac
gate="scripts/dev/gate.sh"
[[ "${need}" == release ]] && gate+=" --release"
if [[ ! -f "${stamp}" ]] || { [[ "${need}" == release ]] && ! grep -qx 'mode=release' "${stamp}"; }; then
  echo "push.sh: refused. Tree ${tree:0:12} (commit ${sha:0:12}) has no ${need} gate stamp." >&2
  echo "Check out that commit and run:" >&2
  echo >&2
  echo "    ${gate}" >&2
  exit 1
fi

if [[ "${branch}" == develop ]] && ((!supersede)); then
  repo="$(git remote get-url origin | sed -E 's#(\.git)?$##; s#^.*github\.com[:/]##')"
  if ! running="$(gh run list --repo "${repo}" --branch develop --workflow ci.yml \
    --status in_progress --json databaseId,url --jq '.[].url')"; then
    echo "push.sh: refused. Could not ask GitHub whether a Fast suite run on develop is in progress." >&2
    exit 1
  fi
  if [[ -n "${running}" ]]; then
    echo "push.sh: refused. A Fast suite run on develop is in progress:" >&2
    echo "${running}" >&2
    echo "Wait for it, or pass --supersede to push anyway." >&2
    exit 1
  fi
fi

args=(origin "${sha}:refs/heads/${branch}")
if [[ "${branch}" == chore/release-* ]]; then
  # The branch's SHA on origin now; empty when the branch does not exist,
  # which the lease then requires.
  remote="$(git ls-remote origin "refs/heads/${branch}" | cut -f1)"
  args=(--force-with-lease="refs/heads/${branch}:${remote}" "${args[@]}")
fi
echo "Pushing ${sha:0:12} (tree ${tree:0:12}, ${need} gate stamp) to ${branch}."
if ((dry_run)); then
  echo "Dry run; would run: git push ${args[*]}"
  exit 0
fi
exec git push "${args[@]}"
