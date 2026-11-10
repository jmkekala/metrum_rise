#!/usr/bin/env bash
# Writes the release notes for <version> to stdout (docs/release.md): the source commit, and
# whether saves and worlds from the previous release still load. Both formats load only on an
# exact match, so any change in either number breaks them.
# Usage: release_notes.sh <version>   (needs the full history and tags)
set -euo pipefail

version=$1
sha=${GITHUB_SHA:-$(git rev-parse HEAD)}
repo=${GITHUB_REPOSITORY:-jmkekala/metrum_rise}
previous=$(git tag --list 'v*' --sort=-v:refname | grep -vx "v$version" | head -1 || true)

save_format() {
  git show "$1:rust/src/simulation/save/schema.rs" | sed -nE 's/^pub const SAVE_VERSION: i64 = ([0-9]+);/\1/p'
}
world_format() {
  git show "$1:rust/src/simulation/world_definition.rs" | sed -nE 's/^const WORLD_DEFINITION_FORMAT_VERSION: i64 = ([0-9]+);/\1/p'
}

echo "Built from commit [\`${sha:0:7}\`](https://github.com/$repo/commit/$sha); source code: https://github.com/$repo/tree/$sha"
echo
if [ -z "$previous" ]; then
  echo "First release."
else
  for kind in save world; do
    before=$("${kind}_format" "$previous")
    after=$("${kind}_format" "$sha")
    label=$([ "$kind" = save ] && echo "Saves" || echo "Worlds")
    if [ "$before" = "$after" ]; then
      echo "- $label from ${previous#v} still load (format $after)."
    else
      echo "- **$label from ${previous#v} will not load** (format $before → $after)."
    fi
  done
fi
echo
echo "The builds are not signed. Windows shows a SmartScreen warning (More info → Run anyway); on macOS, allow the app under System Settings → Privacy & Security."
