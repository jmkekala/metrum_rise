#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Drafts the bundled pack's vehicles (`TOOLS-42`) from one commit of a metrum_models checkout, one
# role file of this folder per model, recording the commit in each ledger (docs/vehicles.md
# "Bundled pack"). The commit's `vehicles/` tree is extracted to a temporary folder, so files the
# models session has not committed are never read. A draft never replaces an existing asset:
# remove godot/bootstrap/mods/vehicles/assets/<id> and its model_sources/<id>.json first to
# re-import a model.
#
#   tools/vehicle_roles/import.sh <metrum_models checkout> <commit> [--write]
set -euo pipefail
checkout="$1"
commit="$(git -C "$checkout" rev-parse --verify "$2^{commit}")"
shift 2
root="$(cd "$(dirname "$0")/../.." && pwd)"
models="$(mktemp -d)"
trap 'rm -rf "$models"' EXIT
git -C "$checkout" archive "$commit" vehicles | tar -x -C "$models"
status=0
for role in "$root"/tools/vehicle_roles/*.json; do
    name="$(basename "$role" .json)"
    sidecar="$(find "$models/vehicles" -path "*/$name/export/$name.model.json" | head -n 1)"
    if [ -z "$sidecar" ]; then
        echo "import: no sidecar for $name in $commit" >&2
        status=1
        continue
    fi
    python3 "$root/tools/model_draft.py" --sidecar "$sidecar" --role "$role" \
        --mods "$root/godot/bootstrap/mods" --source-commit "$commit" "$@" || status=$?
done
exit "$status"
