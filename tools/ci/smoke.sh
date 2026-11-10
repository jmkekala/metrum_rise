#!/usr/bin/env bash
# Release smoke test for an unpacked export (docs/release.md). For each bundled world, on a fresh
# profile, it starts a new game through tools/ci/smoke_driver.gd and then requires:
#   - the driver's "SMOKE result=ok" (a real SimulationNode in the gameplay scene),
#   - no ERROR/WARNING lines except Godot's release-template popup messages (UI-11),
#   - first-run packs and worlds byte-identical to the export's bootstrap manifest,
#   - no Git LFS pointer files among them.
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
worlds=${SMOKE_WORLDS:-"test kuopio_324km2_10m"}
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
for world in $worlds; do
  log=$work/$world.log
  world_failures=$failures
  fail() { echo "FAIL [$platform $world]: $*"; failures=$((failures + 1)); }
  if [ "$platform" = linux ]; then
    profile=$work/profile-$world
    data="$profile/data/godot/app_userdata/Metrum Rise"
    (cd "$game" && env SMOKE_WORLD="user://worlds/$world.sqlite" SMOKE_FRAMES="$frames" \
      XDG_DATA_HOME="$profile/data" XDG_CONFIG_HOME="$profile/config" \
      timeout "$limit" ./MetrumRise.x86_64 --headless) > "$log.raw" 2>&1
  else
    data="$WINEPREFIX/drive_c/users/$USER/AppData/Roaming/Godot/app_userdata/Metrum Rise"
    rm -rf "$data"
    (cd "$game" && env SMOKE_WORLD="user://worlds/$world.sqlite" SMOKE_FRAMES="$frames" \
      timeout "$limit" "${WINE:-wine}" MetrumRise.exe --headless) > "$log.raw" 2>&1
  fi
  status=$?
  tr -d '\r' < "$log.raw" > "$log"
  echo "== $platform $world: exit $status"

  [ "$status" -eq 0 ] || fail "exit status $status"
  grep -q '^SMOKE result=ok' "$log" || fail "the driver did not report a running game"
  unexpected=$(grep -E '^(ERROR|WARNING|SCRIPT ERROR|USER ERROR|USER WARNING)' "$log" | grep -vE "$allowed")
  [ -z "$unexpected" ] || fail "unexpected log lines:"$'\n'"$unexpected"
  (cd "$data" 2>/dev/null && find mods worlds -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum) \
    > "$work/seeded.sha256"
  diff "$manifest" "$work/seeded.sha256" > "$work/seeded.diff" \
    || fail "first-run content differs from the bootstrap:"$'\n'"$(head -20 "$work/seeded.diff")"
  pointers=$(grep -rlI -m1 '^version https://git-lfs' "$data" 2>/dev/null)
  [ -z "$pointers" ] || fail "Git LFS pointer files:"$'\n'"$pointers"

  if [ "$failures" -gt "$world_failures" ]; then
    echo "---- last 60 log lines ($world) ----"
    tail -60 "$log"
  fi
done

if [ "$failures" -gt 0 ]; then
  echo "Smoke test FAILED ($failures problems)."
  exit 1
fi
echo "Smoke test passed: $platform, worlds: $worlds."
