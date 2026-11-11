#!/usr/bin/env bash
# Writes the release notes for <version> to stdout (docs/release.md): the source commit, and
# whether saves from the previous release still load. A save loads only on an exact
# `SAVE_VERSION` match, so any change breaks it. Worlds need no line: a world definition of any
# older format still loads (`WORLD_DEFINITION_FORMAT_VERSION` is a maximum).
# Usage: release_notes.sh <version>   (needs the full history and tags)
set -euo pipefail

version=$1
sha=${GITHUB_SHA:-$(git rev-parse HEAD)}
repo=${GITHUB_REPOSITORY:-jmkekala/metrum_rise}
previous=$(git tag --list 'v*' --sort=-v:refname | grep -vx "v$version" | head -1 || true)

save_format() {
  git show "$1:rust/src/simulation/save/schema.rs" | sed -nE 's/^pub const SAVE_VERSION: i64 = ([0-9]+);/\1/p'
}

echo "Built from commit [\`${sha:0:7}\`](https://github.com/$repo/commit/$sha); source code: https://github.com/$repo/tree/$sha"
echo
if [ -z "$previous" ]; then
  echo "First release."
else
  before=$(save_format "$previous")
  after=$(save_format "$sha")
  if [ "$before" = "$after" ]; then
    echo "- Saves from ${previous#v} still load (format $after)."
  else
    echo "- **Saves from ${previous#v} will not load** (format $before → $after)."
  fi
fi
echo
echo "The builds are not signed. Windows shows a SmartScreen warning (More info → Run anyway); on macOS, choose Open Anyway under System Settings → Privacy & Security."
