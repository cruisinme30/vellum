#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Checks a built website (ADR 0027) before it is published:
#   - the landing page and the docs home carry the Content Security Policy,
#     run no script but the site's theme.js (the light and dark toggle,
#     D308) and use no inline scripts or styles, which that policy blocks;
#   - every image on them has alt text;
#   - every local link and file they point to exists, and so does every
#     link in the site bar at the top of each help book page;
#   - the build left no placeholder unfilled, on any page;
#   - every help book page has the light and dark toggle and loads only the
#     site's own fonts, not mdBook's (D307, D308);
#   - every help book page ends with the feedback and edit links (D257);
#   - every page, the landing page, docs home and every book, has the one
#     site bar, the same links in the same order with only their paths and
#     the current section differing (ADR 0071, D310);
#   - no Mermaid, architecture or chart fence is published as source: the
#     rich blocks were drawn (D309).
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
  grep -qiE "script-src 'self'" <<<"${html}" || fail "${name} does not allow its theme script"
  while IFS= read -r script; do
    grep -qE '^<script src="(\.\./)?theme\.js"></script>$' <<<"${script}" || fail "${name} has a script other than theme.js: ${script}"
  done < <(grep -oiE '<script[^>]*>(</script>)?' <<<"${html}")
  grep -qE '[[:space:]]style="' <<<"${html}" && fail "${name} has an inline style attribute"
  grep -qE '<!-- (BOOKS|SITEBAR|COUNT:[a-z]+) -->' <<<"${html}" && fail "${name} has an unfilled placeholder"
  while IFS= read -r img; do
    grep -q ' alt="' <<<"${img}" || fail "${name} has an image without alt text: ${img}"
  done < <(grep -oE '<img[^>]*>' <<<"${html}")
  check_links "${page}" "${html}"
done

# site_bar <page>: the page's site bar, and its shape with paths, the
# current-section mark and the release links taken out.
site_bar() {
  sed -n '/<header class="vellum-site">/,/<\/header>/p' "$1"
}
bar_shape() {
  sed -E -e 's/(href|src)="[^"]*"/\1=""/g' -e 's/ aria-current="page"//g' -e 's#<span class="vellum-versions">.*</span>##' -e 's/^[[:space:]]+//' <<<"$1" | grep -v '^$'
}
home_bar="$(site_bar "${site}/index.html")"
[[ -n "${home_bar}" ]] || fail "index.html has no site bar"
home_shape="$(bar_shape "${home_bar}")"
docs_bar="$(site_bar "${site}/docs/index.html")"
[[ "$(bar_shape "${docs_bar}")" == "${home_shape}" ]] || fail "docs/index.html has a different site bar from the landing page"

# The site bar on every help book page.
while IFS= read -r -d '' page; do
  bar="$(site_bar "${page}")"
  if [[ -z "${bar}" ]]; then
    fail "${page#"${site}"/} has no site bar"
    continue
  fi
  [[ "$(bar_shape "${bar}")" == "${home_shape}" ]] || fail "${page#"${site}"/} has a different site bar from the landing page"
  grep -qE 'class="language-(mermaid|architecture|chart|timeline|roadmap)"' "${page}" && fail "${page#"${site}"/} shows a diagram as source instead of drawing it"
  check_links "${page}" "${bar}"
  # Code spans may name a placeholder on purpose (ADR 0027 does).
  sed -E 's#<code>[^<]*</code>##g' "${page}" | grep -qE '@(ROOT|HOME|DOCS|GUIDES|VERSIONS|FONTS|CUR-[a-z]+)@' && fail "${page#"${site}"/} has an unfilled placeholder"
  grep -q 'class="theme-toggle"' <<<"${bar}" || fail "${page#"${site}"/} has no light and dark toggle"
  grep -qE 'href="[^"]*fonts/fonts-[0-9a-f]+\.css"' "${page}" && fail "${page#"${site}"/} still loads mdBook's own fonts"
  case "${page}" in
    */decisions/*) ;;
    *) grep -q 'class="vellum-feedback"' "${page}" || fail "${page#"${site}"/} has no feedback links" ;;
  esac
done < <(find "${site}/docs" -mindepth 2 -name '*.html' -not -name 'toc.html' -not -name 'print.html' -not -name '404.html' -print0)

if ((failures > 0)); then
  echo "check-site: ${failures} problem(s) found" >&2
  exit 1
fi
echo "check-site: ${site} is fine."
