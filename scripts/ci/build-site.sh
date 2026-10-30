#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds the project website into a folder (default: _site):
#   - the landing page from site/ with its fonts, images and video,
#   - every help book docs/<name>.book/ with mdBook, at docs/<name>/,
#   - the architecture decision records docs/adr/ as one more book, at
#     docs/decisions/,
#   - the docs home at docs/, listing every guide's parts and chapters,
#   - the books of the newest release tags at docs/v/<version>/, so readers
#     of an older release find its guide (ADR 0024, D257).
# Every help page ends with "Was this page helpful?" and "Edit this page"
# links; neither sends anything until the reader submits a GitHub issue.
# Vellum Markdown is turned into plain CommonMark on the way: front matter
# is dropped and ::: container fences are unwrapped, so unmodified mdBook
# can build the books (ADR 0027, D111).
#
# Usage: scripts/ci/build-site.sh [output-dir]
# Environment: MDBOOK (default: mdbook), SITE_BASE (default: /vellum/), the
# path the site is served under, used only by mdBook's 404 page;
# SITE_VERSIONS (default: 5), how many release tags (vX.Y.Z) get their own
# copy of the books, newest first; 0 builds only the current books.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$(realpath -m "${1:-${root}/_site}")"
mdbook="${MDBOOK:-mdbook}"
base="${SITE_BASE:-/vellum/}"
max_versions="${SITE_VERSIONS:-5}"
# Where the books being built come from on GitHub, for edit links.
edit_ref="develop"
repo="https://github.com/cruisinme30/vellum"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

# to_commonmark <file>: rewrites a Vellum Markdown page in place. Drops a
# leading YAML front matter block and the opening and closing lines of
# ::: containers (a details title becomes a bold line), rewrites relative
# links for the published layout (<name>.book/ becomes <name>/, README.md
# becomes index.md), and turns the
# front matter title into the page heading when the page has none. It
# never touches anything inside a code fence, where
# the help books show Vellum Markdown source on purpose.
to_commonmark() {
  awk '
    # Published books live at docs/<name>/ and mdBook writes README.md
    # chapters as index.html, so relative links follow both renames.
    function fix_target(t) {
      if (t ~ /^[A-Za-z][A-Za-z0-9+.-]*:/ || t ~ /^[\/#]/) return t
      gsub(/\.book\//, "/", t)
      if (t ~ /(^|\/)README\.md(#|$)/) sub(/README\.md/, "index.md", t)
      return t
    }
    function fix_links(s,   out, link) {
      out = ""
      while (match(s, /\]\([^) \t]+/)) {
        link = substr(s, RSTART + 2, RLENGTH - 2)
        out = out substr(s, 1, RSTART + 1) fix_target(link)
        s = substr(s, RSTART + RLENGTH)
      }
      out = out s
      if (match(out, /^ ? ? ?\[[^]]+\]:[ \t]*/) && RLENGTH < length(out)) {
        link = substr(out, RSTART + RLENGTH); sub(/[ \t].*$/, "", link)
        out = substr(out, 1, RLENGTH) fix_target(link) substr(out, RLENGTH + 1 + length(link))
      }
      return out
    }
    NR == 1 && $0 == "---" { front = 1; next }
    front {
      if ($0 == "---" || $0 == "...") front = 0
      else if ($0 ~ /^title:/) { heading = $0; sub(/^title:[ \t]*/, "", heading); gsub(/^["\047]|["\047]$/, "", heading) }
      next
    }
    heading != "" && $0 !~ /^[ \t]*$/ {
      if ($0 !~ /^# /) print "# " heading "\n"
      heading = ""
    }
    {
      if (fence == "") {
        if (match($0, /^ ? ? ?(```+|~~~+)/)) {
          run = substr($0, RSTART, RLENGTH); sub(/^ +/, "", run)
          fence = run; print; next
        }
        if ($0 ~ /^ ? ? ?:::+/) {
          title = $0; sub(/^ *:+ *details +/, "", title)
          if (title != $0 && title != "") print "**" title "**\n"
          next
        }
        $0 = fix_links($0)
      } else {
        line = $0; sub(/^ +/, "", line); sub(/[ \t]+$/, "", line)
        if (substr(line, 1, 1) == substr(fence, 1, 1) && line ~ /^(`+|~+)$/ && length(line) >= length(fence)) fence = ""
      }
      print
    }
  ' "$1" >"$1.tmp"
  mv "$1.tmp" "$1"
}

# url_encode <text>: percent-encodes text for a URL query.
url_encode() {
  local text="$1" out="" c i
  for ((i = 0; i < ${#text}; i++)); do
    c="${text:i:1}"
    case "${c}" in
      [A-Za-z0-9.~_-]) out+="${c}" ;;
      *) out+="$(printf '%%%02X' "'${c}")" ;;
    esac
  done
  printf '%s' "${out}"
}

# add_feedback <file> <book> <page-path> <version>: ends a page with "Was
# this page helpful?" and "Edit this page". Yes and No open a prefilled
# GitHub issue; nothing is sent unless the reader submits it, and the site
# has no analytics (D257).
add_feedback() {
  local file="$1" book="$2" page="$3" version="$4" title issue
  title="$(yaml_title "${file}")"
  title="${title:-${page}}"
  issue="${repo}/issues/new?labels=docs-feedback&amp;title=$(url_encode "Docs feedback: ${title}")"
  local body="Page: ${book}/${page} (${version})"
  {
    echo
    echo '<div class="vellum-feedback" role="group" aria-label="Feedback on this page">'
    echo "<span>Was this page helpful?</span>"
    echo "<a href=\"${issue}%20(helpful)&amp;body=$(url_encode "${body}"$'\n\nWhat helped most?')\">Yes</a>"
    echo "<a href=\"${issue}%20(not%20helpful)&amp;body=$(url_encode "${body}"$'\n\nWhat were you looking for, and what was missing or wrong?')\">No</a>"
    echo "<a class=\"vellum-edit\" href=\"${repo}/edit/${edit_ref}/docs/${book}.book/${page}\">Edit this page</a>"
    echo '</div>'
  } >>"${file}"
}

# yaml_title <file>: the top-level title: value of a YAML file or front
# matter, without quotes.
yaml_title() {
  sed -n 's/^title:[[:space:]]*//p' "$1" | head -n 1 | sed -e 's/^["'\'']//' -e 's/["'\'']$//'
}

html_escape() {
  sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g' <<<"$1"
}

# fix_html <dir>: patches mdBook 0.5.4's HTML where it fails WCAG 2.2 AA
# or breaks its own scripts (#249):
#   - the sidebar toggle is a <label> carrying button ARIA: give it the role;
#   - code blocks and tables that scroll sideways become focusable;
#   - task-list checkboxes get a name;
#   - mdBook's web fonts are dropped: they load without font-display, so
#     text appears late and the page shifts (book.css uses the site's own
#     fonts, which never swap in late);
#   - headings named like a JavaScript module global (exports, module,
#     define, require) get a suffix: the browser exposes an element id as
#     a window property, and mdBook's search and copy libraries then take
#     the page for CommonJS and fail.
fix_html() {
  find "$1" -name '*.html' -exec sed -i -E \
    -e '/<link rel="stylesheet" href="fonts\/fonts-[0-9a-f]*\.css">/d' \
    -e 's/<label id="mdbook-sidebar-toggle" /<label id="mdbook-sidebar-toggle" role="button" /' \
    -e 's/<pre([^>]*)><code( |>)/<pre\1><code tabindex="0"\2/g' \
    -e 's/<div class="table-wrapper">/<div class="table-wrapper" tabindex="0">/g' \
    -e 's/<input disabled="" type="checkbox" checked="">/<input disabled="" type="checkbox" checked="" aria-label="Done">/g' \
    -e 's/<input disabled="" type="checkbox">/<input disabled="" type="checkbox" aria-label="To do">/g' \
    -e 's/id="(exports|module|define|require)"/id="\1-section"/g' \
    -e 's/href="([^"]*)#(exports|module|define|require)"/href="\1#\2-section"/g' {} +
}

# version_links <name> <current>: the site bar's version links for book
# <name>, with <current> ("latest" or a version) marked. Relative to the
# docs home, which the header reaches as @DOCS@.
version_links() {
  local name="$1" current="$2" version href
  ((${#versions[@]} > 0)) || return 0
  printf '<span class="vellum-versions">Version:'
  for version in latest "${versions[@]}"; do
    if [[ "${version}" == latest ]]; then href="${name}/index.html"; else href="v/${version}/${name}/index.html"; fi
    if [[ "${version}" == "${current}" ]]; then
      printf ' <a href="{{ path_to_root }}@DOCS@%s" aria-current="true">%s</a>' "${href}" "${version}"
    else
      printf ' <a href="{{ path_to_root }}@DOCS@%s">%s</a>' "${href}" "${version}"
    fi
  done
  printf '</span>'
}

# build_book <name> <title> <staged-src> [<version>]: runs mdBook on a
# staged source, into docs/<name>/, or docs/v/<version>/<name>/ for an
# older release. Only the current books are listed on the docs home.
build_book() {
  local name="$1" title="$2" src="$3" version="${4:-latest}" dir dest up docs toml_title
  dir="${work}/build-${version}-${name}"
  if [[ "${version}" == latest ]]; then
    dest="docs/${name}"
  else
    dest="docs/v/${version}/${name}"
  fi
  # From a book's folder up to the site root, and to the docs home.
  up="$(sed -E 's#[^/]+#..#g' <<<"${dest}")/"
  docs="${up%../}"
  toml_title="${2//\\/\\\\}"
  toml_title="${toml_title//\"/\\\"}"
  find "${src}" -name '*.md' -print0 | while IFS= read -r -d '' page; do
    to_commonmark "${page}"
  done
  # to_commonmark points every README.md link, SUMMARY.md's included, at
  # index.md, which mdBook publishes as index.html; rename the files to
  # match. (Before this, mdBook made an empty index.md for each part: the
  # published parts showed only their title.)
  find "${src}" -name README.md -print0 | while IFS= read -r -d '' page; do
    mv "${page}" "${page%README.md}index.md"
  done
  mkdir -p "${dir}/theme"
  cp "${root}/site/book.css" "${dir}/theme/vellum.css"
  version_links "${name}" "${version}" >"${dir}/versions.html"
  awk -v links="${dir}/versions.html" '
    /@VERSIONS@/ { while ((getline line < links) > 0) print line; close(links); next }
    { print }
  ' "${root}/site/book-header.hbs" | sed -e "s#@ROOT@#${up}#g" -e "s#@DOCS@#${docs}#g" >"${dir}/theme/header.hbs"
  cat >"${dir}/book.toml" <<EOF
[book]
title = "${toml_title}"
language = "en"
src = "$(realpath --relative-to="${dir}" "${src}")"

# A chapter missing from SUMMARY.md's targets fails the build instead of
# being published empty.
[build]
create-missing = false

[output.html]
site-url = "${base}${dest}/"
git-repository-url = "${repo}"
default-theme = "light"
preferred-dark-theme = "navy"
additional-css = ["theme/vellum.css"]

# No "run" button: it calls play.rust-lang.org from every page with Rust
# code, which leaks readers to a third party and fails offline (#249).
[output.html.playground]
runnable = false
EOF
  "${mdbook}" build "${dir}" --dest-dir "${out}/${dest}"
  fix_html "${out}/${dest}"
  [[ "${version}" == latest ]] && books+=("${name}"$'\t'"${title}")
  return 0
}

# build_guides <docs-dir> <version>: every help book in <docs-dir> (a
# checkout's docs/), the three guides first, plus its decision records.
build_guides() {
  local docs_dir="$1" version="$2" stage="${work}/src-$2" book name title page
  mkdir -p "${stage}"
  mapfile -t book_dirs < <(
    for name in user admin developer; do
      [[ -d "${docs_dir}/${name}.book" ]] && echo "${docs_dir}/${name}.book"
    done
    find "${docs_dir}" -mindepth 1 -maxdepth 1 -type d -name '*.book' \
      -not -name user.book -not -name admin.book -not -name developer.book | sort
  )
  for book in "${book_dirs[@]}"; do
    [[ -f "${book}/SUMMARY.md" ]] || continue
    name="$(basename "${book}" .book)"
    title="$(yaml_title "${book}/book.yaml" 2>/dev/null || true)"
    cp -R "${book}" "${stage}/${name}"
    while IFS= read -r -d '' page; do
      add_feedback "${page}" "${name}" "${page#"${stage}/${name}/"}" "${version}"
    done < <(find "${stage}/${name}" -name '*.md' -not -name SUMMARY.md -print0)
    build_book "${name}" "${title:-${name}}" "${stage}/${name}" "${version}"
  done
  build_decisions "${docs_dir}/adr" "${version}"
}

rm -rf "${out}"
mkdir -p "${out}/docs"
cp "${root}/site/style.css" "${root}/site/favicon.svg" "${root}/site/install.sh" "${out}/"
cp -R "${root}/site/fonts" "${root}/site/img" "${root}/site/media" "${out}/"

# build_decisions <adr-dir> <version>: the architecture decision records,
# as a generated book.
build_decisions() {
  local adr src adrs=("$1"/[0-9]*.md)
  [[ -e "${adrs[0]}" ]] || return 0
  src="${work}/decisions-$2"
  mkdir -p "${src}"
  {
    echo "# Summary"
    echo
    echo "[Architecture decisions](README.md)"
    echo
    for adr in "${adrs[@]}"; do
      # 0000-template.md is the blank form for new records, not a decision.
      [[ "$(basename "${adr}")" == 0000-* ]] && continue
      cp "${adr}" "${src}/"
      echo "- [$(yaml_title "${adr}")]($(basename "${adr}"))"
    done
  } >"${src}/SUMMARY.md"
  cat >"${src}/README.md" <<'EOF'
# Architecture decisions

Every lasting decision about Vellum is recorded here as an architecture
decision record: the context, the decision and its consequences. Decision
numbers (D1, D2 and so on) are unique across all records.
EOF
  build_book decisions "Vellum architecture decisions" "${src}" "$2"
}


books=()

# Release tags, newest first: each gets a copy of its own books.
versions=()
if ((max_versions > 0)) && git -C "${root}" rev-parse --git-dir >/dev/null 2>&1; then
  mapfile -t versions < <(git -C "${root}" tag --list 'v[0-9]*' --sort=-version:refname | head -n "${max_versions}")
fi

# The current books, then each release's.
build_guides "${root}/docs" latest
for version in "${versions[@]}"; do
  mkdir -p "${work}/tag-${version}"
  git -C "${root}" archive "${version}" docs | tar -x -C "${work}/tag-${version}"
  build_guides "${work}/tag-${version}/docs" "${version}"
done

# summary_html <name> <summary-file>: the contents of one guide for the
# docs home: each part with its chapters and their sections, as links into
# the built book. Chapters before the first part are left out (the guide's
# own link covers the introduction); chapters after a "---" rule are
# listed under "Reference".
summary_html() {
  awk -v book="$1" '
    function esc(t) { gsub(/&/, "\\&amp;", t); gsub(/</, "\\&lt;", t); gsub(/>/, "\\&gt;", t); gsub(/"/, "\\&quot;", t); return t }
    function href(t) {
      if (t ~ /(^|\/)README\.md$/) sub(/README\.md$/, "index.html", t)
      else sub(/\.md$/, ".html", t)
      return book "/" t
    }
    function link(line,   title, target) {
      match(line, /\[[^]]+\]/); title = substr(line, RSTART + 1, RLENGTH - 2)
      match(line, /\]\([^)]+\)/); target = substr(line, RSTART + 2, RLENGTH - 3)
      return "<a href=\"" esc(href(target)) "\">" esc(title) "</a>"
    }
    function close_sub() { if (sub_open) { printf "</ul>"; sub_open = 0 } }
    function close_item() { close_sub(); if (item_open) { print "</li>"; item_open = 0 } }
    function close_part() { close_item(); if (part_open) { print "            </ul>\n          </div>"; part_open = 0 } }
    function open_part(title) {
      close_part()
      print "          <div class=\"part\">\n            <h4>" esc(title) "</h4>\n            <ul>"
      part_open = 1
    }
    /^# / { title = substr($0, 3); if (title != "Summary") { parts = 1; open_part(title) } next }
    /^---[ \t]*$/ { if (parts) { open_part("Reference"); suffix = 1 } next }
    (parts && /^- \[/) || (suffix && /^\[/) {
      close_item(); printf "              <li>%s", link($0); item_open = 1; next
    }
    parts && /^[ \t]+- \[/ && item_open {
      if (!sub_open) { printf "<ul>"; sub_open = 1 }
      printf "<li>%s</li>", link($0); next
    }
    END { close_part() }
  ' "$2"
}

# describe <name>: the one-line summary under a guide's title.
describe() {
  case "$1" in
    user) echo "For everyone: write pages and books, organise data, track work, plan your day and work with Claude." ;;
    admin) echo "For whoever runs Vellum: install, configure, secure, back up, upgrade and monitor it." ;;
    developer) echo "For builders: the architecture, the Markdown dialect, the APIs, the MCP server and how to contribute." ;;
    *) echo "Read the guide." ;;
  esac
}

# The docs home: a card per guide with its parts and chapters.
cards="${work}/cards.html"
{
  echo '        <ul class="books">'
  for entry in "${books[@]}"; do
    name="${entry%%$'\t'*}"
    [[ "${name}" == decisions ]] && continue
    title="$(html_escape "${entry#*$'\t'}")"
    echo '          <li class="book">'
    printf '          <h3><a href="%s/index.html">%s</a></h3>\n' "${name}" "${title}"
    printf '          <p>%s</p>\n' "$(describe "${name}")"
    echo '          <div class="parts">'
    summary_html "${name}" "${root}/docs/${name}.book/SUMMARY.md"
    echo '          </div>'
    echo '          </li>'
  done
  echo '        </ul>'
} >"${cards}"

# Counts shown on both pages, so they never go stale.
pages="$(find "${root}/docs" -path '*.book/*' -name '*.md' -not -name SUMMARY.md | wc -l)"
crates="$(find "${root}/crates" -mindepth 2 -maxdepth 2 -name Cargo.toml | wc -l)"
decisions="$(find "${root}/docs/adr" -maxdepth 1 -name '[0-9]*.md' -not -name '0000-*' | wc -l)"

# fill <template> <output>: puts in the guide cards and the counts.
fill() {
  awk -v cards="${cards}" -v pages="${pages// /}" -v crates="${crates// /}" -v decisions="${decisions// /}" '
    /<!-- BOOKS -->/ { while ((getline line < cards) > 0) print line; close(cards); next }
    {
      gsub(/<!-- COUNT:pages -->/, pages)
      gsub(/<!-- COUNT:crates -->/, crates)
      gsub(/<!-- COUNT:decisions -->/, decisions)
      print
    }
  ' "$1" >"$2"
}
fill "${root}/site/index.html" "${out}/index.html"
fill "${root}/site/docs.html" "${out}/docs/index.html"

echo "Built the site in ${out} with ${#books[@]} book(s) and ${pages} help pages."
