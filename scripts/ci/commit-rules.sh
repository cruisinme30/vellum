#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Shared commit rules, sourced by CI and by .githooks/commit-msg.

readonly CONVENTIONAL_RE='^(build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)(\([a-z0-9._/-]+\))?!?: [^ ].{0,71}$'

# check_subject <subject>: succeeds for a Conventional Commits subject.
check_subject() {
  [[ "$1" =~ ${CONVENTIONAL_RE} ]]
}

# check_signoff <message> <name> <email>: succeeds when the message has a
# Signed-off-by trailer for that author.
check_signoff() {
  grep -qxF "Signed-off-by: $2 <$3>" <<<"$1"
}

readonly BRANCH_RE='^((build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)/[a-z0-9][a-z0-9._-]*|dependabot/.+)$'

# check_branch <name>: succeeds for a <type>/<topic> branch name.
check_branch() {
  [[ "$1" =~ ${BRANCH_RE} ]]
}
