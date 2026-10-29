#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Keeps the public "Vellum 1.0" GitHub Project in sync with the task issues.
# Every task issue is one project item. Status comes from the issue state and
# its status/* labels, Phase from the milestone, Area from the area/* label,
# Priority from the priority/* label, and Owner thread, ADR and PR from the
# task form's sections. Option descriptions come from the milestone and label
# descriptions, so the project explains itself.
#
# Idempotent: it adds missing items, creates or extends fields, and writes
# only the values that changed, so it is safe to run on every issue event.
#
# Environment: GH_TOKEN (needs the "project" scope; GITHUB_TOKEN cannot write
# user-owned projects), GH_REPO (owner/name), PROJECT_OWNER, PROJECT_NUMBER.
# GraphQL queries use $variables inside single quotes on purpose.
# shellcheck disable=SC2016
set -euo pipefail

REPO="${GH_REPO:?GH_REPO is required}"
OWNER="${PROJECT_OWNER:?PROJECT_OWNER is required}"
num="${PROJECT_NUMBER:?PROJECT_NUMBER is required}"
BATCH=10
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

# gqlv QUERY VARIABLES_JSON: one GraphQL call with typed variables.
gqlv() {
  local v="${2:-}"; [[ -n "$v" ]] || v='{}'
  jq -n --arg q "$1" --argjson v "$v" '{query:$q,variables:$v}' | gh api graphql --input -
}

PID="$(gh project view "${num}" --owner "${OWNER}" --format json --jq .id)"

# ---- Desired values per task issue -------------------------------------------------
gh issue list --repo "$REPO" --label task --state all --limit 1000 \
  --json id,number,url,state,labels,milestone,body > "$TMP/raw.json"
jq '
  def sec($h): ([(.body // "") | capture("(?s)### " + $h + "[ \\t]*\\r?\\n(?<v>.*?)(\\r?\\n### |\\r?\\n---|\\z)").v] | first // "")
               | gsub("^\\s+|\\s+$"; "");
  def labels: [.labels[].name];
  map({
    id, number,
    Status: (if .state == "CLOSED" then "Done"
             elif (labels | index("status/blocked")) then "Blocked"
             elif (labels | index("status/in-progress")) then "In Progress"
             else "Todo" end),
    Phase: (.milestone.title // ""),
    Area: ([labels[] | select(startswith("area/")) | ltrimstr("area/")] | first // ""),
    "Owner thread": sec("Owner thread"),
    Priority: ([labels[] | select(startswith("priority/")) | ltrimstr("priority/") | ascii_upcase] | first // ""),
    ADR: (sec("ADR") | (split("\n")[0] // "") | sub("^[-*]\\s*"; "") | gsub("\\[(?<t>[^\\]]*)\\]\\((?<u>[^)]*)\\)"; "\(.t) \(.u)")),
    PR: (sec("Pull requests") | if test("^none( yet)?$"; "i") then "" else . end
         | split("\n") | map(gsub("^\\s*[-*]\\s*|\\s+$"; "") | select(length > 0)) | join(", "))
  })' "$TMP/raw.json" > "$TMP/want.json"
echo "Task issues: $(jq length "$TMP/want.json")"

# ---- Fields --------------------------------------------------------------------------
load_fields() {
  FIELDS="$(gqlv 'query($id:ID!){node(id:$id){... on ProjectV2{fields(first:100){nodes{
      ... on ProjectV2Field{id name dataType}
      ... on ProjectV2IterationField{id name dataType}
      ... on ProjectV2SingleSelectField{id name dataType options{id name color description}}}}}}}' \
    "$(jq -n --arg id "$PID" '{id:$id}')" | jq -c '.data.node.fields.nodes | map(select(.name))')"
}

# ensure_field NAME TEXT|SINGLE_SELECT [OPTIONS_JSON=[{name,color,description}]]
ensure_field() {
  local name="$1" type="$2" want="${3:-[]}" cur
  cur="$(jq -c --arg n "${name}" '.[] | select(.name == $n)' <<<"${FIELDS}")"
  if [[ -n "${cur}" && "$(jq -r .dataType <<<"${cur}")" != "${type}" ]]; then
    gqlv 'mutation($id:ID!){deleteProjectV2Field(input:{fieldId:$id}){clientMutationId}}' \
      "$(jq -c '{id:.id}' <<<"${cur}")" >/dev/null
    echo "Replaced field ${name} ($(jq -r .dataType <<<"${cur}") -> ${type})"
    cur=""
  fi
  if [[ -z "${cur}" ]]; then
    if [[ "${type}" == TEXT ]]; then
      gqlv 'mutation($p:ID!,$n:String!){createProjectV2Field(input:{projectId:$p,dataType:TEXT,name:$n}){clientMutationId}}' \
        "$(jq -n --arg p "${PID}" --arg n "${name}" '{p:$p,n:$n}')" >/dev/null
    else
      gqlv 'mutation($p:ID!,$n:String!,$o:[ProjectV2SingleSelectFieldOptionInput!]!){createProjectV2Field(input:{projectId:$p,dataType:SINGLE_SELECT,name:$n,singleSelectOptions:$o}){clientMutationId}}' \
        "$(jq -n --arg p "${PID}" --arg n "${name}" --argjson w "${want}" '{p:$p,n:$n,o:($w | map({name, color, description}))}')" >/dev/null
    fi
    echo "Created field ${name}"
  elif [[ "${type}" == SINGLE_SELECT ]] && jq -e --argjson w "${want}" '
      (.options | map({key:.name, value:(.description // "")}) | from_entries) as $h
      | any($w[]; $h[.name] != .description)' <<<"${cur}" >/dev/null; then
    # Add missing options and refresh descriptions. Existing options keep their
    # ids (so item values survive) and their colors (so UI tweaks survive).
    gqlv 'mutation($f:ID!,$o:[ProjectV2SingleSelectFieldOptionInput!]!){updateProjectV2Field(input:{fieldId:$f,singleSelectOptions:$o}){clientMutationId}}' \
      "$(jq -c --argjson w "${want}" '
          (.options | map({key:.name, value:.}) | from_entries) as $h
          | {f:.id, o:(($w | map(. as $o | ($h[$o.name] // {}) as $e
                                 | {name:$o.name, color:($e.color // $o.color), description:$o.description}
                                   + (if $e.id then {id:$e.id} else {} end)))
                      + [.options[] | select(.name as $x | $w | map(.name) | index($x) | not)
                         | {id, name, color, description:(.description // "")}])}' <<<"${cur}")" >/dev/null
    echo "Updated options of ${name}"
  fi
}

palette='["BLUE","GREEN","YELLOW","ORANGE","RED","PINK","PURPLE","GRAY"]'

# Phase: one option per milestone, described by the milestone (minus its footer).
phases="$(gh api "repos/${REPO}/milestones?state=all&per_page=100" \
  | jq -c --argjson p "${palette}" --slurpfile w "${TMP}/want.json" '
      (map({key:.title, value:((.description // "") | sub("\\s*\\n---[\\s\\S]*$"; "") | gsub("^\\s+|\\s+$"; ""))}) | from_entries) as $d
      | (map(.title) + [$w[0][].Phase]) | map(select(. != "")) | unique
      | to_entries | map({name:.value, color:$p[.key % 8], description:($d[.value] // "")})')"
# Area: one option per area/* label, described by the label.
areas="$(gh label list --repo "${REPO}" --limit 500 --json name,description \
  | jq -c --slurpfile w "${TMP}/want.json" '
      (map(select(.name | startswith("area/")) | {key:(.name | ltrimstr("area/")), value:(.description // "")}) | from_entries) as $d
      | ($d | keys) + [$w[0][].Area] | map(select(. != "")) | unique
      | map({name:., color:"GRAY", description:($d[.] // "")})')"
owners="$(jq -c '[.[]."Owner thread" | select(. != "")] | unique
  | map({name:., color:"GRAY", description:"Work thread that builds these tasks"})' "${TMP}/want.json")"
prios="$(jq -c '[.[].Priority | select(. != "")] + ["P0"] | unique
  | map({name:., color:"RED", description:(if . == "P0" then "Required for 1.0. Every requirement is P0; there is no MVP" else "" end)})' "${TMP}/want.json")"
statuses='[
  {"name":"Todo","color":"GRAY","description":"Not started"},
  {"name":"In Progress","color":"YELLOW","description":"A pull request that closes this task is open (status/in-progress)"},
  {"name":"Blocked","color":"RED","description":"Waiting on another task or a decision (status/blocked)"},
  {"name":"Done","color":"GREEN","description":"Merged into develop; the issue is closed"}]'

load_fields
ensure_field Status SINGLE_SELECT "$statuses"
ensure_field Phase SINGLE_SELECT "$phases"
ensure_field Area SINGLE_SELECT "$areas"
ensure_field "Owner thread" SINGLE_SELECT "$owners"
ensure_field Priority SINGLE_SELECT "$prios"
ensure_field ADR TEXT
ensure_field PR TEXT
load_fields

# ---- Items ---------------------------------------------------------------------------
load_items() {
  gh api graphql --paginate -f pid="$PID" -f query='query($pid:ID!,$endCursor:String){node(id:$pid){... on ProjectV2{
      items(first:100,after:$endCursor){pageInfo{hasNextPage endCursor} nodes{id content{... on Issue{id}}
        fieldValues(first:50){nodes{
          ... on ProjectV2ItemFieldTextValue{text field{... on ProjectV2FieldCommon{name}}}
          ... on ProjectV2ItemFieldSingleSelectValue{name field{... on ProjectV2FieldCommon{name}}}}}}}}}}' \
  | jq -s -c '[.[].data.node.items.nodes[]] | map({id, content:(.content.id // ""),
      values:([.fieldValues.nodes[] | select(.field) | {key:.field.name, value:(.text // .name // "")}] | from_entries)})'
}

# run_batched FILE: one mutation per line, sent BATCH at a time as aliased mutations.
run_batched() {
  [[ -s "$1" ]] || return 0
  rm -f "$TMP"/chunk.*
  split -l "$BATCH" "$1" "$TMP/chunk."
  for c in "$TMP"/chunk.*; do
    # Adds and value updates are idempotent, so a failed batch is simply retried.
    local q try
    q="mutation{$(awk '{printf "m%d: %s\n", NR, $0}' "${c}")}"
    for try in 1 2 3 4; do
      gqlv "$q" >/dev/null && break
      [[ $try -lt 4 ]] || { echo "Batch failed 4 times; re-run the script to resume." >&2; return 1; }
      sleep $((try * 5))
    done
    sleep 1
  done
}

ITEMS="$(load_items)"
jq -r --argjson items "$ITEMS" --arg p "$PID" '
  ($items | map(.content)) as $have
  | .[] | select(.id as $x | $have | index($x) | not)
  | "addProjectV2ItemById(input:{projectId:\($p|tojson),contentId:\(.id|tojson)}){item{id}}"' \
  "$TMP/want.json" > "$TMP/add.txt"
echo "Adding $(wc -l < "$TMP/add.txt" | tr -d ' ') new items"
run_batched "$TMP/add.txt"

ITEMS="$(load_items)"
jq -r --argjson items "$ITEMS" --argjson fields "$FIELDS" --arg p "$PID" '
  ($items | map({key:.content, value:.}) | from_entries) as $by
  | ($fields | map({key:.name, value:.}) | from_entries) as $f
  | .[] | . as $i | $by[$i.id] as $it | select($it)
  | ("Status", "Phase", "Area", "Owner thread", "Priority", "ADR", "PR") as $n
  | ($i[$n] // "") as $want | ($it.values[$n] // "") as $cur
  | select($want != $cur)
  | $f[$n] as $fd
  | "projectId:\($p|tojson),itemId:\($it.id|tojson),fieldId:\($fd.id|tojson)" as $base
  | if $want == "" then "clearProjectV2ItemFieldValue(input:{\($base)}){clientMutationId}"
    elif $fd.dataType == "SINGLE_SELECT" then
      ([$fd.options[] | select(.name == $want) | .id] | first) as $o
      | select($o) | "updateProjectV2ItemFieldValue(input:{\($base),value:{singleSelectOptionId:\($o|tojson)}}){clientMutationId}"
    else "updateProjectV2ItemFieldValue(input:{\($base),value:{text:\($want|tojson)}}){clientMutationId}" end' \
  "$TMP/want.json" > "$TMP/set.txt"
echo "Updating $(wc -l < "$TMP/set.txt" | tr -d ' ') field values"
run_batched "$TMP/set.txt"

echo
echo "Done: $(jq length <<<"$(load_items)") items. https://github.com/users/$OWNER/projects/$num"
