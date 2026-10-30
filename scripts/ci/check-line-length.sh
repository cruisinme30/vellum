#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Fails on lines over 80 characters (ADR 0074, D320).
#
#   scripts/ci/check-line-length.sh --staged   lines the staged change adds
#                                              (the pre-commit hook)
#   scripts/ci/check-line-length.sh [FILE...]  every line of these files, or
#                                              of every tracked file
#
# Exempt, because they can't be wrapped:
#   - files marked `line-length=off` in .gitattributes: lockfiles, generated
#     files, licence texts, test data whose exact bytes are checked and
#     Fluent messages;
#   - binary files;
#   - a line that fits once its URLs are taken out.
set -euo pipefail

readonly MAX=80
root="$(git rev-parse --show-toplevel)"
cd "${root}"

# checked FILE...: the files that are not exempt.
checked() {
  printf '%s\n' "$@" |
    git check-attr --stdin line-length |
    awk -F': ' '$3 != "off" { print $1 }'
}

# Perl counts characters, not bytes (-CSD reads UTF-8). Input lines are
# "<path>\t<line number>\t<text>"; prints the ones that are too long.
too_long() {
  perl -CSD -ne '
    chomp;
    my ($path, $number, $text) = split /\t/, $_, 3;
    next if length($text) <= '"${MAX}"';
    (my $short = $text) =~ s{[a-z][a-z0-9+.-]*://[^\s"'"'"'<>)`]+}{}g;
    printf "%s:%d: %d characters\n", $path, $number, length($text)
      if length($short) > '"${MAX}"';
  '
}

if [[ "${1:-}" == "--staged" ]]; then
  mapfile -t files < <(git diff --cached --name-only --diff-filter=ACMR)
  ((${#files[@]} > 0)) || exit 0
  mapfile -t files < <(checked "${files[@]}")
  ((${#files[@]} > 0)) || exit 0
  long="$(
    git diff --cached --no-color --no-ext-diff -U0 -- "${files[@]}" |
      perl -CSD -ne '
        if (/^\+\+\+ b\/(.*)$/) { $path = $1; next }
        if (/^@@ -\S+ \+(\d+)/) { $number = $1; next }
        if (/^\+(.*)$/) { print "$path\t$number\t$1\n"; $number++ }
      ' |
      too_long
  )"
else
  if (($# > 0)); then files=("$@"); else mapfile -t files < <(git ls-files); fi
  mapfile -t files < <(checked "${files[@]}")
  ((${#files[@]} > 0)) || exit 0
  mapfile -t files < <(grep -Il '' -- "${files[@]}" 2>/dev/null || true)
  ((${#files[@]} > 0)) || exit 0
  long="$(
    perl -CSD -ne 'chomp; print "$ARGV\t$.\t$_\n"; close ARGV if eof' \
      -- "${files[@]}" | too_long
  )"
fi

if [[ -n "${long}" ]]; then
  printf '%s\n' "${long}"
  count="$(wc -l <<<"${long}" | tr -d ' ')"
  echo "check-line-length: ${count} line(s) over ${MAX} characters." \
    "Wrap them (rustfmt and Prettier do code; strings and comments are" \
    "yours), or see ADR 0074 for what is exempt." >&2
  exit 1
fi
