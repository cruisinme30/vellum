#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Syncs the public "Vellum 1.0" GitHub Project with every `task` issue: adds
# missing issues, keeps the Status, Phase, Area, Owner thread, Priority, ADR
# and PR fields in step with each issue, and writes only what changed.
# Environment: GH_TOKEN, a token with the `project` scope (the default
# GITHUB_TOKEN cannot write a user-owned project). Needs gh and jq.
# GraphQL queries name their variables with $; they must not expand.
# shellcheck disable=SC2016
set -euo pipefail

OWNER=cruisinme30
REPO=cruisinme30/vellum
TITLE="Vellum 1.0"
BATCH=10
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

# gqlv QUERY VARIABLES_JSON: one GraphQL call with typed variables.
gqlv() {
  local v="${2:-}"; [[ -n "$v" ]] || v='{}'
  jq -n --arg q "$1" --argjson v "$v" '{query:$q,variables:$v}' | gh api graphql --input -
}

num="$(gh project list --owner "$OWNER" --limit 100 --format json --jq ".projects[] | select(.title==\"$TITLE\") | .number" | head -1)"
if [[ -z "$num" ]]; then
  num="$(gh project create --owner "$OWNER" --title "$TITLE" --format json --jq .number)"
  echo "Created project #$num"
fi
gh project link "$num" --owner "$OWNER" --repo "$REPO" >/dev/null 2>&1 || true
PID="$(gh project view "$num" --owner "$OWNER" --format json --jq .id)"

# ---- Desired values per task issue ----
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

# ---- Fields ----
load_fields() {
  FIELDS="$(gqlv 'query($id:ID!){node(id:$id){... on ProjectV2{fields(first:100){nodes{
      ... on ProjectV2Field{id name dataType}
      ... on ProjectV2IterationField{id name dataType}
      ... on ProjectV2SingleSelectField{id name dataType options{id name color description}}}}}}}' \
    "$(jq -n --arg id "$PID" '{id:$id}')" | jq -c '.data.node.fields.nodes | map(select(.name))')"
}

# ensure_field NAME TEXT|SINGLE_SELECT [OPTIONS_JSON=[{name,color}]]
ensure_field() {
  local name="$1" type="$2" want="${3:-[]}" cur
  cur="$(jq -c --arg n "$name" '.[] | select(.name == $n)' <<<"$FIELDS")"
  if [[ -n "$cur" && "$(jq -r .dataType <<<"$cur")" != "$type" ]]; then
    gqlv 'mutation($id:ID!){deleteProjectV2Field(input:{fieldId:$id}){clientMutationId}}' \
      "$(jq -c '{id:.id}' <<<"$cur")" >/dev/null
    echo "Replaced field $name ($(jq -r .dataType <<<"$cur") -> $type)"
    cur=""
  fi
  if [[ -z "$cur" ]]; then
    if [[ "$type" == TEXT ]]; then
      gqlv 'mutation($p:ID!,$n:String!){createProjectV2Field(input:{projectId:$p,dataType:TEXT,name:$n}){clientMutationId}}' \
        "$(jq -n --arg p "$PID" --arg n "$name" '{p:$p,n:$n}')" >/dev/null
    else
      gqlv 'mutation($p:ID!,$n:String!,$o:[ProjectV2SingleSelectFieldOptionInput!]!){createProjectV2Field(input:{projectId:$p,dataType:SINGLE_SELECT,name:$n,singleSelectOptions:$o}){clientMutationId}}' \
        "$(jq -n --arg p "$PID" --arg n "$name" --argjson w "$want" '{p:$p,n:$n,o:($w|map({name,color,description:""}))}')" >/dev/null
    fi
    echo "Created field $name"
  elif [[ "$type" == SINGLE_SELECT ]] && jq -e --argjson w "$want" '([.options[].name]) as $h | any($w[]; .name as $x | $h | index($x) | not)' <<<"$cur" >/dev/null; then
    # Add the missing options, keeping the existing ones (ids and colors).
    gqlv 'mutation($f:ID!,$o:[ProjectV2SingleSelectFieldOptionInput!]!){updateProjectV2Field(input:{fieldId:$f,singleSelectOptions:$o}){clientMutationId}}' \
      "$(jq -c --argjson w "$want" '
          (.options | map({key:.name, value:.}) | from_entries) as $h
          | {f:.id, o:(($w | map($h[.name] // {name, color, description:""}))
                      + [.options[] | select(.name as $x | $w | map(.name) | index($x) | not)]
                      | map({name, color, description:(.description // "")} + (if .id then {id} else {} end)))}' <<<"$cur")" >/dev/null
    echo "Added options to $name"
  fi
}

palette='["BLUE","GREEN","YELLOW","ORANGE","RED","PINK","PURPLE","GRAY"]'
opts() { jq -c --argjson p "$palette" 'to_entries | map({name:.value, color:$p[.key % 8]})'; }

phases="$(gh api "repos/$REPO/milestones?state=all&per_page=100" --jq '[.[].title]' \
  | jq -c --slurpfile w "$TMP/want.json" '. + [$w[0][].Phase] | map(select(. != "")) | unique' | opts)"
areas="$(gh label list --repo "$REPO" --limit 500 --json name --jq '[.[].name | select(startswith("area/")) | ltrimstr("area/")]' \
  | jq -c 'unique | map({name:., color:"GRAY"})')"
owners="$(jq -c '[.[]."Owner thread" | select(. != "")] | unique | map({name:., color:"GRAY"})' "$TMP/want.json")"
prios="$(jq -c '[.[].Priority | select(. != "")] + ["P0"] | unique | map({name:., color:"RED"})' "$TMP/want.json")"
statuses='[{"name":"Todo","color":"GRAY"},{"name":"In Progress","color":"YELLOW"},{"name":"Blocked","color":"RED"},{"name":"Done","color":"GREEN"}]'

load_fields
# The first version of this script made Owner a text field; Owner thread replaces it.
old_owner="$(jq -c '.[] | select(.name == "Owner" and .dataType == "TEXT")' <<<"$FIELDS")"
if [[ -n "$old_owner" ]]; then
  gqlv 'mutation($id:ID!){deleteProjectV2Field(input:{fieldId:$id}){clientMutationId}}' "$(jq -c '{id:.id}' <<<"$old_owner")" >/dev/null
  echo "Removed old text field Owner"
fi
ensure_field Status SINGLE_SELECT "$statuses"
ensure_field Phase SINGLE_SELECT "$phases"
ensure_field Area SINGLE_SELECT "$areas"
ensure_field "Owner thread" SINGLE_SELECT "$owners"
ensure_field Priority SINGLE_SELECT "$prios"
ensure_field ADR TEXT
ensure_field PR TEXT
load_fields

# ---- Items ----
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
    q="mutation{$(awk '{printf "m%d: %s\n", NR, $0}' "$c")}"
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
