#!/usr/bin/env bash
# Release smoke test for an unpacked export (docs/release.md). For each bundled world, on a fresh
# profile, it starts a new game through tools/ci/smoke_driver.gd and then requires:
#   - the driver's "SMOKE result=ok" (a real SimulationNode in the gameplay scene),
#   - no ERROR/WARNING lines except Godot's release-template popup messages (UI-11),
#   - first-run packs and worlds byte-identical to the export's bootstrap manifest,
#   - no Git LFS pointer files among them,
#   - an install receipt (user://bundled.toml) naming each bundled pack and world as the game's,
#     at the digest of its installed copy (so the manifest compiled into the library matches
#     the exported bundle).
# Then the update case (TOOLS-49): the game starts on a profile prepared from the first fresh one
# as an older release would have left it, with a receipt naming the older digests: an untouched
# older pack and world (one file changed, one added and one removed in the pack) must be
# replaced and end byte-equal to the bundle, a pack the player edited must stay as it was, and
# the receipt must record the new digests, the edited pack as the player's.
# It also requires the licence, credits and third-party licence files beside the executable.
# Run it where the build checkout is absent, so a path baked in at compile time cannot resolve.
# Usage: smoke.sh linux|windows <game-dir> <bootstrap.sha256>
set -uo pipefail

if [ $# -ne 3 ] || [ ! -d "$2" ] || [ ! -f "$3" ]; then
  echo "usage: $0 linux|windows <game-dir> <bootstrap.sha256> (both must exist)" >&2
  exit 2
fi
platform=$1
game=$(cd "$2" && pwd)
manifest=$(realpath "$3")
here=$(cd "$(dirname "$0")" && pwd)
worlds=${SMOKE_WORLDS:-"test kuopio_18432m_8m"}
frames=${SMOKE_FRAMES:-900}
# A driver that never quits (it failed to load) leaves the game idling in the menu until this.
limit=${SMOKE_TIMEOUT:-300}
work=$(mktemp -d)
trap 'rm -f "$game/override.cfg"; rm -rf "$work"' EXIT

# UI-11: Godot's release templates log these whenever a popup opens or closes (godot#87626).
allowed="^ERROR: (Attempt to disconnect a nonexistent connection from 'root:<Window#[0-9]+>'\. Signal: '(focus_entered|tree_exited)', callable: ''\.|Signal '(focus_entered|tree_exited)' is already connected to given callable '' in that object\.)$"

case $platform in
  linux)
    driver=$here/smoke_driver.gd
    ;;
  windows)
    driver="Z:$here/smoke_driver.gd"
    export WINEPREFIX=$work/wine WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="
    "${WINE:-wine}" wineboot -i > "$work/wineboot.log" 2>&1 || { cat "$work/wineboot.log"; exit 1; }
    ;;
  *)
    echo "usage: $0 linux|windows <game-dir> <bootstrap.sha256>" >&2
    exit 2
    ;;
esac
printf '[autoload]\n\nSmokeDriver="*%s"\n' "$driver" > "$game/override.cfg"

failures=0
fail() { echo "FAIL [$platform $name]: $*"; failures=$((failures + 1)); }
for notice in LICENSE.md CREDITS.md THIRD-PARTY-LICENSES.txt; do
  if [ ! -s "$game/$notice" ]; then
    echo "FAIL [$platform]: the package has no $notice"
    failures=$((failures + 1))
  fi
done

# The bundled entries (worlds/<name>, mods/<name>), sorted as the receipt sorts them.
entries=$(cut -c67- "$manifest" | cut -d/ -f1,2 | LC_ALL=C sort -u)

# Sets $data to an empty user data folder for <name> ($profile on Linux).
empty_profile() {
  if [ "$platform" = linux ]; then
    profile=$work/profile-$1
    data="$profile/data/godot/app_userdata/Metrum Rise"
    rm -rf "$profile"
  else
    data="$WINEPREFIX/drive_c/users/$USER/AppData/Roaming/Godot/app_userdata/Metrum Rise"
    rm -rf "$data"
  fi
}

# Starts a new game on <world> in $data and checks the exit, the driver's report and the log.
start_game() {
  log=$work/$name.log
  if [ "$platform" = linux ]; then
    (cd "$game" && env SMOKE_WORLD="user://worlds/$1.sqlite" SMOKE_FRAMES="$frames" \
      XDG_DATA_HOME="$profile/data" XDG_CONFIG_HOME="$profile/config" \
      timeout "$limit" ./MetrumRise.x86_64 --headless) > "$log.raw" 2>&1
  else
    (cd "$game" && env SMOKE_WORLD="user://worlds/$1.sqlite" SMOKE_FRAMES="$frames" \
      timeout "$limit" "${WINE:-wine}" MetrumRise.exe --headless) > "$log.raw" 2>&1
  fi
  local status=$?
  tr -d '\r' < "$log.raw" > "$log"
  echo "== $platform $name: exit $status"

  [ "$status" -eq 0 ] || fail "exit status $status"
  grep -q '^SMOKE result=ok' "$log" || fail "the driver did not report a running game"
  local unexpected
  unexpected=$(grep -E '^(ERROR|WARNING|SCRIPT ERROR|USER ERROR|USER WARNING)' "$log" | grep -vE "$allowed")
  [ -z "$unexpected" ] || fail "unexpected log lines:"$'\n'"$unexpected"
}

# `sha256sum` lines of every file under the given paths of $data, sorted as the manifest is.
installed_files() {
  (cd "$data" 2>/dev/null && find "$@" -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum)
}

# The digest the game records for an entry (rust/src/assets/bundled.rs): a world's SHA-256, or
# for a pack folder the SHA-256 of its own sorted `sha256sum` lines over every file not
# `.`-prefixed (at any level) and not `*.import`, paths relative with `/`.
digest() {
  if [ -d "$1" ]; then
    (cd "$1" && find . -name '.?*' -prune -o -type f ! -name '*.import' -printf '%P\0' \
      | LC_ALL=C sort -z | xargs -0 -r sha256sum | sha256sum | cut -d' ' -f1)
  else
    sha256sum < "$1" | cut -d' ' -f1
  fi
}

# `<path> <revision> <owner>` per entry of the receipt in $data.
receipt_rows() {
  awk -F'"' '/^\[entries\./ { path = $2 } /^revision = / { revision = $2 }
    /^owner = / { print path, revision, $2 }' "$data/bundled.toml" 2>/dev/null
}

# Compares the receipt in $data with the expected rows on stdin.
check_receipt() {
  receipt_rows > "$work/$name.receipt"
  diff - "$work/$name.receipt" > "$work/$name.receipt.diff" \
    || fail "the install receipt is not as expected:"$'\n'"$(head -20 "$work/$name.receipt.diff")"
}

bundle=
for world in $worlds; do
  name=$world
  name_failures=$failures
  empty_profile "$name"
  start_game "$world"
  installed_files mods worlds > "$work/seeded.sha256"
  diff "$manifest" "$work/seeded.sha256" > "$work/seeded.diff" \
    || fail "first-run content differs from the bootstrap:"$'\n'"$(head -20 "$work/seeded.diff")"
  pointers=$(grep -rlI -m1 '^version https://git-lfs' "$data" 2>/dev/null)
  [ -z "$pointers" ] || fail "Git LFS pointer files:"$'\n'"$pointers"
  for entry in $entries; do echo "$entry $(digest "$data/$entry") game"; done | check_receipt

  if [ "$failures" -gt "$name_failures" ]; then
    echo "---- last 60 log lines ($name) ----"
    tail -60 "$log"
  elif [ -z "$bundle" ]; then
    # The first passing profile holds the bundle byte for byte: the update case starts from it.
    bundle=$work/bundle
    mkdir -p "$bundle"
    cp -R "$data/mods" "$data/worlds" "$bundle/"
  fi
done

name=update
first_world=${worlds%% *}
packs=$(grep '^mods/' <<< "$entries")
old_pack=$(head -1 <<< "$packs")
edited_pack=$(sed -n 2p <<< "$packs")
old_world=worlds/$first_world.sqlite
if [ -z "$bundle" ]; then
  fail "not run: no fresh profile passed"
elif [ -z "$old_pack" ] || ! grep -qx "$old_world" <<< "$entries"; then
  fail "not run: the bundle needs a pack and the world $old_world"
else
  name_failures=$failures
  empty_profile "$name"
  mkdir -p "$data"
  cp -R "$bundle/mods" "$bundle/worlds" "$data/"
  # The older release's untouched copies: a pack with a file changed, one added and one removed
  # (the last file in path order other than pack.toml), and a world with other bytes.
  removed=$(cd "$data/$old_pack" && find . -type f ! -path ./pack.toml -printf '%P\n' | LC_ALL=C sort | tail -1)
  printf '\n# an older release\n' >> "$data/$old_pack/pack.toml"
  printf 'an older release\n' > "$data/$old_pack/older_release_only.txt"
  [ -z "$removed" ] || rm "$data/$old_pack/$removed"
  printf 'an older release' >> "$data/$old_world"
  # The player's edit to a pack the older release installed; the bundle has changed since.
  if [ -n "$edited_pack" ]; then
    printf '\n# edited by the player\n' >> "$data/$edited_pack/pack.toml"
    installed_files "$edited_pack" > "$work/edited.sha256"
  else
    echo "== $platform $name: one bundled pack, so no player-edited pack"
  fi
  for entry in $entries; do
    if [ "$entry" = "$edited_pack" ]; then
      revision=$(printf 'an older %s' "$entry" | sha256sum | cut -d' ' -f1)
    else
      revision=$(digest "$data/$entry")
    fi
    printf '[entries."%s"]\nrevision = "%s"\nowner = "game"\n\n' "$entry" "$revision"
  done > "$data/bundled.toml"

  start_game "$first_world"
  if [ -n "$edited_pack" ]; then
    installed_files "$edited_pack" | diff "$work/edited.sha256" - > "$work/edited.diff" \
      || fail "the player's $edited_pack changed:"$'\n'"$(head -20 "$work/edited.diff")"
  fi
  installed_files mods worlds | grep -v "  $edited_pack/" > "$work/updated.sha256"
  grep -v "  $edited_pack/" "$manifest" | diff - "$work/updated.sha256" > "$work/updated.diff" \
    || fail "the game's entries differ from the bootstrap after the update:"$'\n'"$(head -20 "$work/updated.diff")"
  for entry in $entries; do
    owner=game
    [ "$entry" != "$edited_pack" ] || owner=player
    echo "$entry $(digest "$bundle/$entry") $owner"
  done | check_receipt
  if [ "$failures" -gt "$name_failures" ]; then
    echo "---- last 60 log lines ($name) ----"
    tail -60 "$log"
  fi
fi

if [ "$failures" -gt 0 ]; then
  echo "Smoke test FAILED ($failures problems)."
  exit 1
fi
echo "Smoke test passed: $platform, worlds: $worlds, and the update case."
