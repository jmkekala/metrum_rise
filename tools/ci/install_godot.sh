#!/usr/bin/env bash
# Installs the official Godot editor and export templates for <version> (docs/release.md): the
# downloads are kept in <dist> (cached between CI runs) and checked against the release's
# SHA512-SUMS. The editor becomes <dist>/godot; templates go where Godot looks for them.
# Usage: install_godot.sh <version> <dist>
set -euo pipefail

version=$1
dist=$2
base=https://github.com/godotengine/godot-builds/releases/download/$version-stable
editor=Godot_v$version-stable_linux.x86_64
templates=Godot_v$version-stable_export_templates.tpz

mkdir -p "$dist"
cd "$dist"
[ -f SHA512-SUMS.txt ] || curl -fsSL -o SHA512-SUMS.txt "$base/SHA512-SUMS.txt"
for file in "$editor.zip" "$templates"; do
  [ -f "$file" ] || curl -fsSL -o "$file" "$base/$file"
  grep -E "  $file\$" SHA512-SUMS.txt | sha512sum -c -
done

unzip -oq "$editor.zip"
mv -f "$editor" godot
target=${XDG_DATA_HOME:-$HOME/.local/share}/godot/export_templates/$version.stable
mkdir -p "$target"
unzip -oqj "$templates" 'templates/*' -d "$target"
