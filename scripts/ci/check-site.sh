#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Checks a built website (ADR 0027) before it is published:
#   - the landing page and the docs home carry the Content Security Policy,
#     run no scripts and use no inline styles, which that policy would block;
#   - every image on them has alt text;
#   - every local link and file they point to exists, and so does every
#     link in the site bar at the top of each help book page;
#   - the build left no placeholder unfilled.
#
# Usage: scripts/ci/check-site.sh [site-dir] (default: _site)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
site="$(realpath -m "${1:-${root}/_site}")"
failures=0

fail() {
  echo "check-site: $*" >&2
  failures=$((failures + 1))
}

# check_target <page> <url>: a local link from <page> must name a file in
# the site. Fragments and queries are ignored; a folder means its index.html.
check_target() {
  local page="$1" url="$2" path
  case "${url}" in
    http://* | https://* | mailto:* | '#'* | '') return 0 ;;
  esac
  path="${url%%#*}"
  path="${path%%\?*}"
  [[ -z "${path}" ]] && return 0
  path="$(realpath -m "$(dirname "${page}")/${path}")"
  [[ -d "${path}" ]] && path="${path}/index.html"
  [[ "${path}" == "${site}"/* ]] || { fail "${page#"${site}"/} links outside the site: ${url}"; return 0; }
  [[ -f "${path}" ]] || fail "${page#"${site}"/} links to a missing file: ${url}"
}

# check_links <page> <html>: every href, src and poster in <html>.
check_links() {
  local page="$1" url
  while IFS= read -r url; do
    check_target "${page}" "${url}"
  done < <(grep -oE '(href|src|poster)="[^"]*"' <<<"$2" | sed -E 's/^[a-z]+="//; s/"$//')
}

[[ -f "${site}/index.html" ]] || { echo "check-site: no site in ${site}; run scripts/ci/build-site.sh first" >&2; exit 1; }

for page in "${site}/index.html" "${site}/docs/index.html"; do
  name="${page#"${site}"/}"
  if [[ ! -f "${page}" ]]; then
    fail "${name} is missing"
    continue
  fi
  html="$(cat "${page}")"
  grep -q 'http-equiv="Content-Security-Policy"' <<<"${html}" || fail "${name} has no Content-Security-Policy"
  grep -qi '<script' <<<"${html}" && fail "${name} contains a script"
  grep -qE '[[:space:]]style="' <<<"${html}" && fail "${name} has an inline style attribute"
  grep -qE '<!-- (BOOKS|COUNT:[a-z]+) -->' <<<"${html}" && fail "${name} has an unfilled placeholder"
  while IFS= read -r img; do
    grep -q ' alt="' <<<"${img}" || fail "${name} has an image without alt text: ${img}"
  done < <(grep -oE '<img[^>]*>' <<<"${html}")
  check_links "${page}" "${html}"
done

# The site bar on every help book page.
while IFS= read -r -d '' page; do
  bar="$(sed -n '/<nav class="vellum-site"/,/<\/nav>/p' "${page}")"
  if [[ -z "${bar}" ]]; then
    fail "${page#"${site}"/} has no site bar"
    continue
  fi
  check_links "${page}" "${bar}"
done < <(find "${site}/docs" -mindepth 2 -name '*.html' -not -name 'toc.html' -not -name 'print.html' -not -name '404.html' -print0)

if ((failures > 0)); then
  echo "check-site: ${failures} problem(s) found" >&2
  exit 1
fi
echo "check-site: ${site} is fine."
