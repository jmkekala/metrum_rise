#!/usr/bin/env bash
# Writes the release notes for <version> to stdout (docs/release.md): the source commit, and
# whether saves from the previous release still load. The line is derived from
# `rust/src/simulation/save/schema.rs` at both commits: a save records the `SAVE_VERSION` it was
# written with, and the loader accepts the current format and each older format that schema.rs
# keeps a `PRE_*_SAVE_VERSION` constant for (`load_from_sqlite` in `save/mod.rs`).
# Not in the notes, because no release can meet them:
#   - Saves and worlds from before the aligned grid are refused whatever their format number
#     (`WorldConfig::validate_stored`); the grid landed before the first release.
#   - A world definition of any older format still loads (`WORLD_DEFINITION_FORMAT_VERSION` is
#     a maximum, every format from 1 up loads), so worlds need no line.
# Usage: release_notes.sh <version>   (needs the full history and tags)
set -euo pipefail

version=$1
sha=${GITHUB_SHA:-$(git rev-parse HEAD)}
repo=${GITHUB_REPOSITORY:-jmkekala/metrum_rise}
previous=$(git tag --list 'v*' --sort=-v:refname | grep -vx "v$version" | head -1 || true)

schema() {
  git show "$1:rust/src/simulation/save/schema.rs"
}
save_format() {
  schema "$1" | sed -nE 's/^pub const SAVE_VERSION: i64 = ([0-9]+);/\1/p'
}
# Every save format the loader at <commit> accepts, one a line.
loadable_formats() {
  schema "$1" | sed -nE 's/^pub const (PRE_[A-Z_]+_)?SAVE_VERSION: i64 = ([0-9]+);/\2/p'
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
  elif loadable_formats "$sha" | grep -qx "$before"; then
    echo "- Saves from ${previous#v} still load (format $before, converted to $after at load)."
  else
    echo "- **Saves from ${previous#v} will not load** (format $before → $after)."
  fi
fi
echo
echo "The builds are not signed. Windows shows a SmartScreen warning (More info → Run anyway); on macOS, choose Open Anyway under System Settings → Privacy & Security."
