#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Shared commit rules, sourced by CI and by .githooks/commit-msg.

readonly COMMIT_TYPES='(build|chore|ci|docs|feat|fix|perf|refactor|revert|'\
'style|test)'
readonly SCOPE='(\([a-z0-9._/-]+\))?!?'
readonly CONVENTIONAL_RE="^${COMMIT_TYPES}${SCOPE}: [^ ].{0,71}\$"

# Bots such as Dependabot write titles we cannot shorten ("... in the
# rust-minor group across 1 directory"), so their subjects keep the type
# rule but have no length limit.
readonly CONVENTIONAL_BOT_RE="^${COMMIT_TYPES}${SCOPE}: [^ ].*\$"

# check_subject <subject>: succeeds for a Conventional Commits subject.
check_subject() {
  [[ "$1" =~ ${CONVENTIONAL_RE} ]]
}

# check_bot_subject <subject>: the same rule without the length limit.
check_bot_subject() {
  [[ "$1" =~ ${CONVENTIONAL_BOT_RE} ]]
}

# check_body <message>: succeeds when a blank line follows the subject and
# a body explains the change (trailers such as Signed-off-by do not count).
check_body() {
  local second body
  second="$(sed -n '2p' <<<"$1")"
  body="$(sed -n '3,$p' <<<"$1" | grep -vE '^[A-Za-z-]+: ' \
    | grep -v '^[[:space:]]*$' || true)"
  [[ -z "${second}" && -n "${body}" ]]
}

# check_signoff <message> <name> <email>: succeeds when the message has a
# Signed-off-by trailer for that author.
check_signoff() {
  grep -qxF "Signed-off-by: $2 <$3>" <<<"$1"
}

readonly BRANCH_RE="^(${COMMIT_TYPES}/[a-z0-9][a-z0-9._-]*|dependabot/.+)\$"

# check_branch <name>: succeeds for a <type>/<topic> branch name.
check_branch() {
  [[ "$1" =~ ${BRANCH_RE} ]]
}
