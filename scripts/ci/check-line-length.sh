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
#   - a line that is a single word, an ESLint next-line directive, or a
#     one-name import (Prettier keeps it on one line);
#   - a line that fits once its URLs and pinned hashes (40 or more hex
#     digits, such as a sha256 digest) are taken out;
#   - Markdown table rows and code examples (also in Rust doc comments),
#     fence openers, headings, and title, alt and caption lines;
#   - a JSON line holding a single string, which JSON can't continue.
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
    # in_fence(path, n): line n of the file is inside a Markdown code fence.
    sub in_fence {
      my ($file, $n) = @_;
      unless (exists $fenced{$file}) {
        my (%lines, $fence);
        if (open my $in, "<:encoding(UTF-8)", $file) {
          while (my $line = <$in>) {
            # Rust doc comments are Markdown too.
            if ($file =~ /\.rs\z/) {
              unless ($line =~ s{^\s*//[/!] ?}{}) { undef $fence; next }
            }
            if (defined $fence) {
              undef $fence if $line =~ /^\s*\Q$fence\E\s*$/;
              $lines{$.} = 1 if defined $fence;
            } elsif ($line =~ /^\s*(```+|~~~+)/) {
              $fence = $1;
            }
          }
        }
        $fenced{$file} = \%lines;
      }
      return $fenced{$file}{$n};
    }
    chomp;
    my ($path, $number, $text) = split /\t/, $_, 3;
    next if length($text) <= '"${MAX}"';
    next if $text =~ /^\s*\S+\s*\z/;    # one word: nowhere to break
    # A directive for the next line cannot span lines, and Prettier keeps
    # an import of one name on one line.
    next if $text =~ m{^\s*// eslint-disable-next-line \S+\z};
    next if $path =~ /\.[cm]?[jt]sx?\z/
      && $text =~ /^import (type )?(\{ \w+ \}|\w+) from "[^"]+";\z/;
    next if $path =~ /\.md\z/ && $text =~ /^\s*\|/;
    # Markdown code examples are text to copy exactly, a heading and a
    # fence opener are one line each, and front matter titles and the
    # key: value lines of data fences are read one line at a time.
    next if $path =~ /\.md\z/
      && ($text =~ /^\s*(```|~~~)/ || $text =~ /^(title|alt|caption): /
        || $text =~ /^#{1,6} /
        || in_fence($path, $number));
    next if $path =~ /\.rs\z/ && $text =~ m{^\s*//[/!] \|};
    next if $path =~ /\.rs\z/ && $text =~ m{^\s*//[/!]}
      && in_fence($path, $number);
    # JSON has no way to continue a string on the next line.
    next if $path =~ /\.(json|webmanifest)\z/
      && $text =~ /^\s*("[^"]*":\s*)?"(?:[^"\\]|\\.)*",?\s*\z/;
    (my $short = $text) =~ s{[a-z][a-z0-9+.-]*://[^\s"'"'"'<>)`]+}{}g;
    $short =~ s{\$\{\w+\}/\S+}{}g;    # a URL built from a variable
    $short =~ s{[0-9a-f]{40,}}{}g;
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
