#!/bin/bash
# Metrum Rise Run Script
#
# Verification:
#   --test               Run the Rust suite and headless Godot bridge regressions
#   --benchmark-road-chunks
#                        Run deterministic Rust generation and Godot upload benchmarks
#   --benchmark-road-chunk-upload
#                        Run only the deterministic Godot upload benchmark
#   --benchmark-gameplay-roads / --benchmark-gameplay-roads-headless
#                        Measure the gameplay workload without profiler overhead
#   --profile-gameplay-roads
#                        Profile the windowed controlled road workload with Samply
#   --profile-gameplay-roads-headless
#                        Profile the same workload with Godot's CPU-only headless renderer
#   --gpu-profile        Add Godot GPU stage diagnostics to a windowed gameplay road workload
#   --replay-road-terrain / --replay-road-terrain-headless
#                        Replay pinned Kuopio terrain cases and audit road profiles (diagnostic)
#
# Debug modes:
#   --debug              General debug logging (stdout)
#   --debug <category>   Category-filtered debug logging (stdout)
#                        Common categories: isect, economy, demand, spawn, road, border, terrain, buildings, visuals, perf
#   --debug road         Road placement timings, committed-road geometry dumps,
#                        terrain/water patch diagnostics (METRUM_DEBUG_SURFACE=1 adds overlay)
#   --debug terrain      Terrain + water patch residency/perf summaries (stdout)
#                        Shows resident patch counts, desired bounds, cull distance, patch
#                        create/remove/upload churn, and average renderer timings while flying.
#   --debug terrain-verbose
#                        Same as terrain plus residency-change logs whenever the patch window moves.
#   --debug terrain-full
#                        Same as terrain plus forced full-world terrain/water residency to compare
#                        steady-state full-map cost against camera-driven patch churn.
#   --debug terrain-lod1
#                        Terrain + water diagnostics with all resident patch meshes forced to LOD1.
#   --debug terrain-full-lod1
#                        Forced full-world terrain/water residency and all patch meshes forced to LOD1.
#   --debug terrain-visual <mode>
#                        Terrain/water material diagnostics. Modes:
#                        patch, lod, height, relief, shore, water-depth, water-lod, water-patch,
#                        water-material, lighting
#   --debug perf         Frame CPU diagnostics by renderer. Emits [DEBUG:perf]
#                        summaries every 0.5 s.
#   --debug buildings    Building-site mesh/material diagnostics (log only).
#   --debug building-sites-visual [mode]
#                        Building-site material-source overlay. Modes: material
#   --debug site-grading
#                        Best combo for building/road yard seams: road geometry dump,
#                        building-site diagnostics, and building-site material overlay.
#   --debug traffic      Traffic/routing + road-network connectivity (stderr)
#   --debug-traffic      Alias for --debug traffic
#                        Shows per-road-placement split details, CCH rebuild connectivity
#                        reports, agent routing decisions, and visual traffic overlay labels.
#   --pedestrian-vat-debug <mode>
#   --pedestrian-vat-debug=<mode>
#                        Pedestrian VAT material debug. Modes:
#                        rest = no VAT animation, uv = vertex IDs,
#                        off = VAT offset magnitude overlay.
#   --debug-world-editor Alias for --debug world-editor
#                        Shows world-editor create/open/save/tool activity (stdout)
#   --debug-sim          Hourly simulation summaries (stdout)
#   --debug visuals      Terrain grass visual debug; defaults to material composite.
#   --debug visual       Alias for --debug visuals.
#   --debug visuals <mode>
#                        Terrain grass visual debug mode. Modes:
#                        raw, macro, mid, micro, fades, material, height, mask,
#                        luminance, footprint
#   --visuals [mode]     Alias for --debug visuals [mode]
#
# Release crash diagnostics:
#   --release defaults METRUM_CRASH_DIAGNOSTICS=1 and writes panic/hang dumps to logs/
#   METRUM_CRASH_DIAGNOSTICS=0 ./run.sh --release disables the background recorder
#   METRUM_HANG_WATCHDOG_MS=0 disables the hang watchdog only
#   METRUM_HANG_ABORT=1 aborts after the first hang dump
#
# Godot import cache:
#   Missing/stale imported resources are repaired once per asset-source state.
#   METRUM_SKIP_GODOT_IMPORT_REPAIR=1 skips the repair pass.
#   METRUM_FORCE_GODOT_IMPORT_REPAIR=1 retries even after a previous attempt.

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GODOT_DIR="$PROJECT_ROOT/godot"
case "$(uname -s)" in
    Linux)
        METRUM_PLATFORM="linux"
        METRUM_LIBRARY_SUFFIX="so"
        ;;
    Darwin)
        METRUM_PLATFORM="darwin"
        METRUM_LIBRARY_SUFFIX="dylib"
        ;;
    *)
        echo "Error: unsupported platform '$(uname -s)'; Metrum Rise supports Linux and macOS." >&2
        exit 2
        ;;
esac
RELEASE=0
TEST=0
ROAD_CHUNK_BENCHMARK=0
ROAD_CHUNK_UPLOAD_BENCHMARK=0
GAMEPLAY_ROAD_PROFILE_MODE=""
GAMEPLAY_ROAD_USE_PROFILER=0
GAMEPLAY_ROAD_GPU_PROFILE=0
DEBUG=0
DEBUG_TRAFFIC=0
DEBUG_SIM=0
DEBUG_BUILDINGS=0
BUILDING_SITE_VISUAL_DEBUG=0
BUILDING_SITE_VISUAL_DEBUG_MODE="material"
VISUAL_DEBUG=0
VISUAL_DEBUG_MODE="material"
TERRAIN_VISUAL_DEBUG=0
TERRAIN_VISUAL_DEBUG_MODE="patch"
PEDESTRIAN_VAT_DEBUG_MODE=""
DEBUG_CATEGORY=""
GODOT_ENGINE_ARGS=()
GODOT_ARGS=()
export RUST_BACKTRACE=1

require_command() {
    local command_name="$1"
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Error: required command '$command_name' is not on PATH." >&2
        return 1
    fi
}

hash_stdin() {
    local hash_output
    local digest
    if [ "$METRUM_PLATFORM" = "linux" ]; then
        if ! require_command sha256sum; then
            return 1
        fi
        if ! hash_output="$(sha256sum)"; then
            echo "Error: sha256sum failed." >&2
            return 1
        fi
    else
        if ! require_command shasum; then
            return 1
        fi
        if ! hash_output="$(shasum -a 256)"; then
            echo "Error: shasum -a 256 failed." >&2
            return 1
        fi
    fi
    digest="${hash_output%%[[:space:]]*}"
    case "$digest" in
        ""|*[!0-9a-f]*)
            echo "Error: SHA-256 command produced an invalid digest." >&2
            return 1
            ;;
    esac
    if [ "${#digest}" -ne 64 ]; then
        echo "Error: SHA-256 command produced an invalid digest." >&2
        return 1
    fi
    printf '%s\n' "$digest"
}

asset_modification_state() {
    if [ "$METRUM_PLATFORM" = "linux" ]; then
        stat -c '%Y:%s' "$1"
    else
        stat -f '%m:%z' "$1"
    fi
}

godot_import_signature_records() {
    while IFS= read -r -d '' source_file; do
        local relative_path="${source_file#"$GODOT_DIR"/}"
        local modification_state
        if ! modification_state="$(asset_modification_state "$source_file")"; then
            echo "Error: could not read modification state for $relative_path." >&2
            return 1
        fi
        # %q keeps each record single-line even for unusual source paths.
        printf '%q\t%s\n' "$relative_path" "$modification_state"
    done < <(
        find "$GODOT_DIR/assets" "$GODOT_DIR/bootstrap" -type f \
            \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' \
            -o -iname '*.exr' -o -iname '*.hdr' \
            -o -iname '*.glb' -o -iname '*.gltf' \) \
            -print0
    )
}

deploy_native_library() {
    local source_library="$1"
    local destination_library="$2"
    local temporary_library
    if [ ! -f "$source_library" ]; then
        echo "Error: Rust build succeeded but did not produce $source_library." >&2
        return 1
    fi
    if ! mkdir -p "$(dirname "$destination_library")"; then
        echo "Error: could not create $(dirname "$destination_library")." >&2
        return 1
    fi
    temporary_library="$(mktemp "${destination_library}.tmp.XXXXXX")" || {
        echo "Error: could not create a temporary GDExtension library." >&2
        return 1
    }
    if ! cp "$source_library" "$temporary_library"; then
        rm -f "$temporary_library"
        echo "Error: could not copy the GDExtension library." >&2
        return 1
    fi
    # Rename within godot/bin so a running Godot process retains its old inode.
    if ! mv -f "$temporary_library" "$destination_library"; then
        rm -f "$temporary_library"
        echo "Error: could not deploy the GDExtension library." >&2
        return 1
    fi
}

run_with_test_timeout() {
    if command -v timeout >/dev/null 2>&1; then
        timeout --kill-after=2s 15s "$@"
        return $?
    fi
    if command -v gtimeout >/dev/null 2>&1; then
        gtimeout --kill-after=2s 15s "$@"
        return $?
    fi
    if ! require_command python3; then
        echo "Error: simulation_speed_input_test needs GNU timeout, gtimeout, or Python 3 for its timeout guard." >&2
        return 127
    fi
    python3 - "$@" <<'PY'
import os
import signal
import subprocess
import sys

command = sys.argv[1:]
try:
    child = subprocess.Popen(command, start_new_session=True)
except OSError as error:
    print("Error: could not start timeout-guarded command: {}".format(error), file=sys.stderr)
    sys.exit(127)

interrupted = []

def forward_signal(signum, _frame):
    if not interrupted:
        interrupted.append(signum)
        try:
            os.killpg(child.pid, signum)
        except ProcessLookupError:
            pass

signal.signal(signal.SIGINT, forward_signal)
signal.signal(signal.SIGTERM, forward_signal)

try:
    status = child.wait(timeout=15)
except subprocess.TimeoutExpired:
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        child.wait(timeout=2)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
    sys.exit(124)

if interrupted:
    sys.exit(128 + interrupted[0])
sys.exit(status)
PY
}

godot_import_metadata_has_missing_outputs() {
    local import_file="$1"
    local dest
    while IFS= read -r dest; do
        local native_path="$GODOT_DIR/${dest#res://}"
        if [ ! -f "$native_path" ]; then
            return 0
        fi
    done < <(grep -o 'res://\.godot/imported/[^"]*' "$import_file" || true)
    return 1
}

godot_import_cache_needs_repair() {
    local root
    for root in "$GODOT_DIR/assets" "$GODOT_DIR/bootstrap"; do
        if [ ! -d "$root" ]; then
            continue
        fi
        while IFS= read -r -d '' source_file; do
            local import_file="${source_file}.import"
            local relative_path="${source_file#"$GODOT_DIR"/}"
            if [ ! -f "$import_file" ]; then
                echo "Godot import metadata missing for $relative_path"
                return 0
            fi
            if [ "$source_file" -nt "$import_file" ]; then
                echo "Godot import metadata stale for $relative_path"
                return 0
            fi
            if godot_import_metadata_has_missing_outputs "$import_file"; then
                echo "Godot imported output missing for $relative_path"
                return 0
            fi
        done < <(
            find "$root" -type f \
                \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' \
                -o -iname '*.exr' -o -iname '*.hdr' \
                -o -iname '*.glb' -o -iname '*.gltf' \) \
                -print0
        )
    done
    return 1
}

godot_import_repair_signature() {
    (
        set -o pipefail
        godot_import_signature_records | LC_ALL=C sort | hash_stdin
    )
}

repair_godot_import_cache_if_needed() {
    if [ "${METRUM_SKIP_GODOT_IMPORT_REPAIR:-0}" = "1" ]; then
        echo "Skipping Godot import cache repair (METRUM_SKIP_GODOT_IMPORT_REPAIR=1)."
        return
    fi
    if godot_import_cache_needs_repair; then
        local signature
        if ! signature="$(godot_import_repair_signature)"; then
            echo "Error: could not create the Godot import repair signature." >&2
            return 1
        fi
        local stamp_file="$GODOT_DIR/.godot/import_repair_attempt.sha256"
        if [ "${METRUM_FORCE_GODOT_IMPORT_REPAIR:-0}" != "1" ] && [ -f "$stamp_file" ] && [ "$(cat "$stamp_file")" = "$signature" ]; then
            echo "Godot import cache still incomplete after previous repair attempt; continuing."
            return
        fi
        echo "Repairing Godot import cache..."
        if godot --headless --path "$GODOT_DIR" --import; then
            printf '%s\n' "$signature" > "$stamp_file"
            if godot_import_cache_needs_repair; then
                echo "Godot import cache is still incomplete; runtime source loaders will be used where available."
            fi
        else
            printf '%s\n' "$signature" > "$stamp_file"
            echo "Godot import cache repair failed; continuing with runtime source loaders where available."
        fi
    fi
}

i=1
while [ $i -le $# ]; do
    arg="${!i}"
    if [ "$arg" = "--release" ]; then
        RELEASE=1
    elif [ "$arg" = "--test" ]; then
        TEST=1
    elif [ "$arg" = "--benchmark-road-chunks" ]; then
        ROAD_CHUNK_BENCHMARK=1
        RELEASE=1
    elif [ "$arg" = "--benchmark-road-chunk-upload" ]; then
        ROAD_CHUNK_UPLOAD_BENCHMARK=1
        RELEASE=1
    elif [ "$arg" = "--replay-road-terrain" ] || [ "$arg" = "--replay-road-terrain-headless" ]; then
        GAMEPLAY_ROAD_PROFILE_MODE="windowed"
        if [ "$arg" = "--replay-road-terrain-headless" ]; then
            GAMEPLAY_ROAD_PROFILE_MODE="headless"
        fi
        export METRUM_GAMEPLAY_BENCHMARK_MATRIX="terrain"
        RELEASE=1
    elif [ "$arg" = "--gpu-profile" ]; then
        GAMEPLAY_ROAD_GPU_PROFILE=1
    elif [ "$arg" = "--profile-gameplay-roads" ]; then
        GAMEPLAY_ROAD_PROFILE_MODE="windowed"
        GAMEPLAY_ROAD_USE_PROFILER=1
        RELEASE=1
    elif [ "$arg" = "--profile-gameplay-roads-headless" ]; then
        GAMEPLAY_ROAD_PROFILE_MODE="headless"
        GAMEPLAY_ROAD_USE_PROFILER=1
        RELEASE=1
    elif [ "$arg" = "--benchmark-gameplay-roads" ]; then
        GAMEPLAY_ROAD_PROFILE_MODE="windowed"
        RELEASE=1
    elif [ "$arg" = "--benchmark-gameplay-roads-headless" ]; then
        GAMEPLAY_ROAD_PROFILE_MODE="headless"
        RELEASE=1
    elif [[ "$arg" == --visuals=* ]]; then
        VISUAL_DEBUG=1
        VISUAL_DEBUG_MODE="${arg#--visuals=}"
    elif [ "$arg" = "--visuals" ]; then
        VISUAL_DEBUG=1
        next_index=$((i + 1))
        if [ $next_index -le $# ]; then
            next_arg="${!next_index}"
            if [[ "$next_arg" != --* ]]; then
                VISUAL_DEBUG_MODE="$next_arg"
                i=$((i + 1))
            fi
        fi
    elif [ "$arg" = "--debug-sim" ]; then
        DEBUG_SIM=1
    elif [ "$arg" = "--headless" ]; then
        GODOT_ENGINE_ARGS+=("$arg")
    elif [ "$arg" = "--quit-after" ]; then
        GODOT_ENGINE_ARGS+=("$arg")
        next_index=$((i + 1))
        if [ $next_index -le $# ]; then
            GODOT_ENGINE_ARGS+=("${!next_index}")
            i=$((i + 1))
        fi
    elif [[ "$arg" == --quit-after=* ]]; then
        GODOT_ENGINE_ARGS+=("$arg")
    elif [ "$arg" = "--debug-traffic" ]; then
        DEBUG_TRAFFIC=1
    elif [ "$arg" = "--pedestrian-vat-debug" ]; then
        next_index=$((i + 1))
        if [ $next_index -le $# ]; then
            PEDESTRIAN_VAT_DEBUG_MODE="${!next_index}"
            i=$((i + 1))
        else
            PEDESTRIAN_VAT_DEBUG_MODE="rest"
        fi
    elif [[ "$arg" == --pedestrian-vat-debug=* ]]; then
        PEDESTRIAN_VAT_DEBUG_MODE="${arg#--pedestrian-vat-debug=}"
    elif [ "$arg" = "--debug-world-editor" ]; then
        DEBUG=1
        DEBUG_CATEGORY="world-editor"
    elif [ "$arg" = "--debug" ]; then
        next_index=$((i + 1))
        if [ $next_index -le $# ]; then
            next_arg="${!next_index}"
            if [[ "$next_arg" != --* ]]; then
                if [ "$next_arg" = "traffic" ]; then
                    DEBUG_TRAFFIC=1
                elif [ "$next_arg" = "site-grading" ] || [ "$next_arg" = "building-site-grading" ]; then
                    DEBUG=1
                    DEBUG_BUILDINGS=1
                    BUILDING_SITE_VISUAL_DEBUG=1
                    DEBUG_CATEGORY="road"
                elif [ "$next_arg" = "buildings" ] || [ "$next_arg" = "building-sites" ]; then
                    DEBUG=1
                    DEBUG_BUILDINGS=1
                    DEBUG_CATEGORY="buildings"
                elif [ "$next_arg" = "building-sites-visual" ] || [ "$next_arg" = "building-visual" ] || [ "$next_arg" = "buildings-visual" ]; then
                    BUILDING_SITE_VISUAL_DEBUG=1
                    mode_index=$((i + 2))
                    if [ $mode_index -le $# ]; then
                        mode_arg="${!mode_index}"
                        if [[ "$mode_arg" != --* ]]; then
                            BUILDING_SITE_VISUAL_DEBUG_MODE="$mode_arg"
                            i=$((i + 1))
                        fi
                    fi
                elif [ "$next_arg" = "visuals" ] || [ "$next_arg" = "visual" ]; then
                    VISUAL_DEBUG=1
                    mode_index=$((i + 2))
                    if [ $mode_index -le $# ]; then
                        mode_arg="${!mode_index}"
                        if [[ "$mode_arg" != --* ]]; then
                            VISUAL_DEBUG_MODE="$mode_arg"
                            i=$((i + 1))
                        fi
                    fi
                elif [[ "$next_arg" == visuals=* ]] || [[ "$next_arg" == visual=* ]]; then
                    VISUAL_DEBUG=1
                    VISUAL_DEBUG_MODE="${next_arg#visuals=}"
                    VISUAL_DEBUG_MODE="${VISUAL_DEBUG_MODE#visual=}"
                elif [ "$next_arg" = "terrain-visual" ]; then
                    DEBUG=1
                    if [ -z "$DEBUG_CATEGORY" ]; then
                        DEBUG_CATEGORY="terrain"
                    fi
                    TERRAIN_VISUAL_DEBUG=1
                    mode_index=$((i + 2))
                    if [ $mode_index -le $# ]; then
                        mode_arg="${!mode_index}"
                        if [[ "$mode_arg" != --* ]]; then
                            TERRAIN_VISUAL_DEBUG_MODE="$mode_arg"
                            i=$((i + 1))
                        fi
                    fi
                elif [[ "$next_arg" == terrain-visual=* ]]; then
                    DEBUG=1
                    if [ -z "$DEBUG_CATEGORY" ]; then
                        DEBUG_CATEGORY="terrain"
                    fi
                    TERRAIN_VISUAL_DEBUG=1
                    TERRAIN_VISUAL_DEBUG_MODE="${next_arg#terrain-visual=}"
                else
                    DEBUG=1
                    DEBUG_CATEGORY="$next_arg"
                fi
                i=$((i + 1))
            else
                DEBUG=1
            fi
        else
            DEBUG=1
        fi
    else
        GODOT_ARGS+=("$arg")
    fi
    i=$((i + 1))
done

if [ "$GAMEPLAY_ROAD_GPU_PROFILE" -eq 1 ] && [ "$GAMEPLAY_ROAD_PROFILE_MODE" != "windowed" ]; then
    echo "Error: --gpu-profile requires a windowed gameplay road workload (for example --benchmark-gameplay-roads)." >&2
    exit 2
fi

case "$VISUAL_DEBUG_MODE" in
    raw|albedo)
        VISUAL_DEBUG_MODE="raw"
        ;;
    macro|mid|micro|fades|fade|visibility|material|composite|height|mask|grass-mask|luminance|luma|brightness|footprint|footprints)
        ;;
    "")
        VISUAL_DEBUG_MODE="material"
        ;;
    *)
        echo "Error: unknown visual debug mode '$VISUAL_DEBUG_MODE'." >&2
        echo "Valid modes: raw, macro, mid, micro, fades, material, height, mask, luminance, footprint" >&2
        exit 2
        ;;
esac

if [ "$VISUAL_DEBUG_MODE" = "fade" ] || [ "$VISUAL_DEBUG_MODE" = "visibility" ]; then
    VISUAL_DEBUG_MODE="fades"
elif [ "$VISUAL_DEBUG_MODE" = "composite" ]; then
    VISUAL_DEBUG_MODE="material"
elif [ "$VISUAL_DEBUG_MODE" = "grass-mask" ]; then
    VISUAL_DEBUG_MODE="mask"
elif [ "$VISUAL_DEBUG_MODE" = "luma" ] || [ "$VISUAL_DEBUG_MODE" = "brightness" ]; then
    VISUAL_DEBUG_MODE="luminance"
elif [ "$VISUAL_DEBUG_MODE" = "footprints" ]; then
    VISUAL_DEBUG_MODE="footprint"
fi

case "$TERRAIN_VISUAL_DEBUG_MODE" in
    patch|patches)
        TERRAIN_VISUAL_DEBUG_MODE="patch"
        ;;
    lod|lods)
        TERRAIN_VISUAL_DEBUG_MODE="lod"
        ;;
    height|relief|shore)
        ;;
    shoreline)
        TERRAIN_VISUAL_DEBUG_MODE="shore"
        ;;
    water|depth|water-depth)
        TERRAIN_VISUAL_DEBUG_MODE="water-depth"
        ;;
    water-lod|water-patch|water-material|water-mat|material-water)
        if [ "$TERRAIN_VISUAL_DEBUG_MODE" = "water-mat" ] || [ "$TERRAIN_VISUAL_DEBUG_MODE" = "material-water" ]; then
            TERRAIN_VISUAL_DEBUG_MODE="water-material"
        fi
        ;;
    lighting|light|sun)
        TERRAIN_VISUAL_DEBUG_MODE="lighting"
        ;;
    "")
        TERRAIN_VISUAL_DEBUG_MODE="patch"
        ;;
    *)
        echo "Error: unknown terrain visual debug mode '$TERRAIN_VISUAL_DEBUG_MODE'." >&2
        echo "Valid modes: patch, lod, height, relief, shore, water-depth, water-lod, water-patch, water-material, lighting" >&2
        exit 2
        ;;
esac

case "$BUILDING_SITE_VISUAL_DEBUG_MODE" in
    material|materials|source|sources)
        BUILDING_SITE_VISUAL_DEBUG_MODE="material"
        ;;
    "")
        BUILDING_SITE_VISUAL_DEBUG_MODE="material"
        ;;
    *)
        echo "Error: unknown building-site visual debug mode '$BUILDING_SITE_VISUAL_DEBUG_MODE'." >&2
        echo "Valid modes: material" >&2
        exit 2
        ;;
esac

case "$PEDESTRIAN_VAT_DEBUG_MODE" in
    rest|uv|off|"")
        ;;
    offset|offsets)
        PEDESTRIAN_VAT_DEBUG_MODE="off"
        ;;
    *)
        echo "Error: unknown pedestrian VAT debug mode '$PEDESTRIAN_VAT_DEBUG_MODE'." >&2
        echo "Valid modes: rest, uv, off" >&2
        exit 2
        ;;
esac

if [ "$DEBUG_CATEGORY" = "road-geometry" ]; then
    echo "Error: --debug road-geometry was removed. Use --debug road." >&2
    exit 2
fi

if [ -z "${METRUM_CRASH_LOG_DIR:-}" ]; then
    export METRUM_CRASH_LOG_DIR="$PROJECT_ROOT/logs"
fi

if [ -n "$GAMEPLAY_ROAD_PROFILE_MODE" ] && [ -z "${METRUM_CRASH_DIAGNOSTICS+x}" ]; then
    export METRUM_CRASH_DIAGNOSTICS=0
fi

if [ $RELEASE -eq 1 ] && [ -z "${METRUM_CRASH_DIAGNOSTICS+x}" ]; then
    export METRUM_CRASH_DIAGNOSTICS=1
fi

if [ -n "${METRUM_CRASH_DIAGNOSTICS:-}" ] && [ "${METRUM_CRASH_DIAGNOSTICS}" != "0" ]; then
    mkdir -p "$METRUM_CRASH_LOG_DIR"
    echo "Crash diagnostics enabled (panic/hang dumps go to $METRUM_CRASH_LOG_DIR)"
fi

if [ $DEBUG -eq 0 ] && [ -n "${METRUM_DEBUG_FILTER:-}" ]; then
    DEBUG=1
    DEBUG_CATEGORY="$METRUM_DEBUG_FILTER"
fi

if [ $DEBUG -eq 1 ]; then
    export METRUM_DEBUG=1
    if [ -n "$DEBUG_CATEGORY" ]; then
        if [ "$DEBUG_CATEGORY" = "road" ]; then
            export METRUM_DEBUG_FILTER="road"
            export METRUM_DEBUG_ROAD_GEOMETRY_DUMP=1
        else
            export METRUM_DEBUG_FILTER="$DEBUG_CATEGORY"
        fi
        echo "Debug logging enabled for category '$DEBUG_CATEGORY' (output goes to stdout)"
        if [ "$DEBUG_CATEGORY" = "road" ]; then
            echo "  Road placement timing summaries enabled."
            echo "  After each committed road refresh: [DEBUG:road] ROAD_GEOMETRY_DUMP_BEGIN ... ROAD_GEOMETRY_DUMP_END"
            echo "  Includes edge geometry, node class/throat diagnostics, compiled loops, and source/visual terrain samples."
            echo "  Also prints terrain/water patch clip, mesh, baseline water diagnostics, and authored fill contributors for road-touched patches."
            echo "  Set METRUM_DEBUG_SURFACE=1 to add the compiled road-surface overlay."
        elif [ "$DEBUG_CATEGORY" = "terrain" ]; then
            export METRUM_DEBUG_TERRAIN=1
            echo "  Terrain flight diagnostics enabled: [DEBUG:terrain] summaries every 0.5 s"
            echo "  Includes resident patch counts, desired bounds, cull distance, and terrain/water timing."
        elif [ "$DEBUG_CATEGORY" = "terrain-verbose" ]; then
            export METRUM_DEBUG_TERRAIN=1
            export METRUM_DEBUG_TERRAIN_VERBOSE=1
            echo "  Terrain flight diagnostics enabled: summaries + residency-change logs."
        elif [ "$DEBUG_CATEGORY" = "terrain-full" ]; then
            export METRUM_DEBUG_TERRAIN=1
            export METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD=1
            echo "  Terrain flight diagnostics enabled with forced full-world terrain/water residency."
            echo "  Use this to compare steady-state full-map cost against camera-driven patch churn."
        elif [ "$DEBUG_CATEGORY" = "terrain-lod1" ]; then
            export METRUM_DEBUG_TERRAIN=1
            export METRUM_DEBUG_TERRAIN_FORCE_LOD1=1
            echo "  Terrain flight diagnostics enabled with all resident terrain/water meshes forced to LOD1."
            echo "  Use this to isolate mesh LOD stitching from material/texture issues."
        elif [ "$DEBUG_CATEGORY" = "terrain-full-lod1" ]; then
            export METRUM_DEBUG_TERRAIN=1
            export METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD=1
            export METRUM_DEBUG_TERRAIN_FORCE_LOD1=1
            echo "  Terrain flight diagnostics enabled with full-world residency and forced LOD1 meshes."
            echo "  Use this to reproduce seam artifacts without camera residency or LOD churn."
        elif [ "$DEBUG_CATEGORY" = "perf" ]; then
            export METRUM_DEBUG_PERF=1
            echo "  Frame CPU perf diagnostics enabled: [DEBUG:perf] summaries every 0.5 s"
            echo "  Reports total renderer CPU per completed frame plus per-renderer averages/maxes."
        elif [ "$DEBUG_CATEGORY" = "buildings" ]; then
            DEBUG_BUILDINGS=1
            echo "  Building-site diagnostics enabled: [DEBUG:buildings] mesh, material, height, and site metadata."
            echo "  Visual overlay is separate: add --debug building-sites-visual material when needed."
        fi
    else
        echo "Debug logging enabled (output goes to stdout)"
    fi
fi
if [ $DEBUG_TRAFFIC -eq 1 ]; then
    export METRUM_DEBUG_TRAFFIC=1
    echo "Traffic/routing + road-network debug logging enabled (output goes to stderr)"
    echo "  Per road placement: [ROAD] split details"
    echo "  After CCH rebuild:  [ROAD_NET] connectivity report (1 line if OK, component list if disconnected)"
    echo "  Visual overlay: car lane/connector debug auto-enabled; press P to toggle."
    echo "  Junction logs: [JUNCTION_ENTER], [JUNCTION_EXIT], [JUNCTION_WAIT], [JUNCTION_MISSING_CONN]"
fi
if [ -n "$PEDESTRIAN_VAT_DEBUG_MODE" ]; then
    export METRUM_DEBUG_PEDESTRIAN_VAT="$PEDESTRIAN_VAT_DEBUG_MODE"
    echo "Pedestrian VAT visual debug enabled: '$PEDESTRIAN_VAT_DEBUG_MODE'"
    echo "  rest disables animation/VAT offsets; rigid sliding is expected."
    echo "  uv colors vertex-id UVs; off/offset shows VAT offset magnitude while applying offsets."
    echo "  Use no --pedestrian-vat-debug flag for normal animated character colors."
fi
if [ $DEBUG_SIM -eq 1 ]; then
    export METRUM_DEBUG_SIM=1
    echo "Simulation console debug enabled (hourly summaries go to stdout)"
fi
if [ $DEBUG_BUILDINGS -eq 1 ]; then
    export METRUM_DEBUG_BUILDINGS=1
    if [ "$DEBUG_CATEGORY" != "buildings" ]; then
        echo "Building-site diagnostics enabled: [DEBUG:buildings] site, edge, material, and grading samples."
    fi
fi
if [ $BUILDING_SITE_VISUAL_DEBUG -eq 1 ]; then
    export METRUM_DEBUG_BUILDING_SITES_VISUAL="$BUILDING_SITE_VISUAL_DEBUG_MODE"
    echo "Building-site visual debug enabled: material source overlay"
    echo "  Ground yards, asphalt surfaces, and concrete surfaces are tinted separately."
fi
if [ $VISUAL_DEBUG -eq 1 ]; then
    export METRUM_DEBUG_TERRAIN_GRASS="$VISUAL_DEBUG_MODE"
    echo "Terrain grass visual debug enabled: '$VISUAL_DEBUG_MODE'"
    echo "  Modes:"
    echo "    raw      Direct Grass002 albedo at terrain UV scale"
    echo "    macro    Large stochastic grass layer"
    echo "    mid      Mid-distance stochastic layer"
    echo "    micro    Close-up grass layer"
    echo "    fades    RGB = macro/mid/micro visibility"
    echo "    material Grass material composite before hillshade/contours"
    echo "    height   Grass002 height map"
    echo "    mask     Where grass detail is allowed"
    echo "    luminance Base/macro/mid/micro/final brightness bands"
    echo "    footprint RGB = footprint, micro visibility, grass mask"
fi
if [ $TERRAIN_VISUAL_DEBUG -eq 1 ]; then
    export METRUM_DEBUG_TERRAIN=1
    export METRUM_DEBUG_TERRAIN_VISUAL="$TERRAIN_VISUAL_DEBUG_MODE"
    echo "Terrain/water visual debug enabled: '$TERRAIN_VISUAL_DEBUG_MODE'"
    echo "  Modes:"
    echo "    patch       Terrain patch identity colors with patch borders"
    echo "    lod         Terrain mesh LOD colors with patch borders"
    echo "    height      Terrain height field"
    echo "    relief      Local terrain relief"
    echo "    shore       Terrain-side shore mask from water depth"
    echo "    water-depth Water depth field on terrain and water"
    echo "    water-lod   Water mesh LOD colors"
    echo "    water-patch Water patch identity colors"
    echo "    water-material Water material bands: tint, alpha, Fresnel, foam, normals"
    echo "    lighting    Sun-facing strength, cascade bands, and water specular mask"
fi

echo "Building Rust library..."
cd "$PROJECT_ROOT/rust"
if ! require_command cargo; then
    exit 2
fi
if [ $RELEASE -eq 1 ]; then
    if ! cargo build --release; then
        echo "Rust build failed!"
        exit 1
    fi
    LIB="target/release/libmetrum_rise.$METRUM_LIBRARY_SUFFIX"
else
    if ! cargo build; then
        echo "Rust build failed!"
        exit 1
    fi
    LIB="target/debug/libmetrum_rise.$METRUM_LIBRARY_SUFFIX"
fi

echo "Deploying library..."
if ! deploy_native_library "$LIB" "$GODOT_DIR/bin/libmetrum_rise.$METRUM_LIBRARY_SUFFIX"; then
    exit 1
fi

echo "Registering GDExtension..."
if ! mkdir -p "$GODOT_DIR/.godot" || \
   ! printf 'res://bin/metrum_rise.gdextension\n' > "$GODOT_DIR/.godot/extension_list.cfg"; then
    echo "Error: could not register the GDExtension with Godot." >&2
    exit 1
fi
if ! require_command godot; then
    exit 2
fi

if ! repair_godot_import_cache_if_needed; then
    exit 1
fi

if [ -n "$GAMEPLAY_ROAD_PROFILE_MODE" ]; then
    if [ "$GAMEPLAY_ROAD_USE_PROFILER" -eq 1 ] && ! command -v samply >/dev/null 2>&1; then
        echo "Error: samply is not installed or not on PATH." >&2
        exit 2
    fi
    # perf_event_paranoid is a Linux perf permission gate; macOS Samply does not use it.
    if [ "$METRUM_PLATFORM" = "linux" ] && [ "$GAMEPLAY_ROAD_USE_PROFILER" -eq 1 ] && [ -r /proc/sys/kernel/perf_event_paranoid ]; then
        PERF_EVENT_PARANOID="$(< /proc/sys/kernel/perf_event_paranoid)"
        if [ "$PERF_EVENT_PARANOID" -gt 1 ]; then
            echo "Error: kernel.perf_event_paranoid=$PERF_EVENT_PARANOID; Samply needs 1 or lower." >&2
            exit 2
        fi
    fi

    GAMEPLAY_RESULTS_DIR="${METRUM_GAMEPLAY_BENCHMARK_OUTPUT_DIR:-$PROJECT_ROOT/benchmark-results}"
    GAMEPLAY_RUN_ID="${METRUM_GAMEPLAY_BENCHMARK_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
    GAMEPLAY_BASE="$GAMEPLAY_RESULTS_DIR/gameplay-roads-$GAMEPLAY_ROAD_PROFILE_MODE-$GAMEPLAY_RUN_ID"
    GAMEPLAY_PROFILE_PATH="${METRUM_GAMEPLAY_BENCHMARK_PROFILE_PATH:-$GAMEPLAY_BASE.profile.json.gz}"
    GAMEPLAY_SYMBOLS_PATH="${GAMEPLAY_PROFILE_PATH%.gz}.syms.json"
    GAMEPLAY_METRICS_PATH="${METRUM_GAMEPLAY_BENCHMARK_METRICS_PATH:-$GAMEPLAY_BASE.metrics.json}"
    GAMEPLAY_LOG_PATH="${METRUM_GAMEPLAY_BENCHMARK_LOG_PATH:-$GAMEPLAY_BASE.godot.log}"
    GAMEPLAY_WORLD_PATH="${METRUM_GAMEPLAY_BENCHMARK_WORLD_PATH:-$PROJECT_ROOT/maps/processed/Kuopio/kuopio_324km2_10m.sqlite}"
    GAMEPLAY_SAMPLE_RATE="${METRUM_GAMEPLAY_BENCHMARK_SAMPLE_RATE:-1000}"

    if [ "${METRUM_GAMEPLAY_BENCHMARK_MATRIX:-paired}" != "paired" ] && \
       [ "${METRUM_GAMEPLAY_BENCHMARK_MATRIX:-paired}" != "scaling" ] && \
       [ "${METRUM_GAMEPLAY_BENCHMARK_MATRIX:-paired}" != "interaction" ] && \
       [ "${METRUM_GAMEPLAY_BENCHMARK_MATRIX:-paired}" != "saved" ] && \
       [ "${METRUM_GAMEPLAY_BENCHMARK_MATRIX:-paired}" != "terrain" ] && \
       [ ! -f "$GAMEPLAY_WORLD_PATH" ]; then
        echo "Error: Kuopio world definition not found at $GAMEPLAY_WORLD_PATH" >&2
        exit 2
    fi
    mkdir -p "$GAMEPLAY_RESULTS_DIR" "$(dirname "$GAMEPLAY_PROFILE_PATH")" "$(dirname "$GAMEPLAY_METRICS_PATH")" "$(dirname "$GAMEPLAY_LOG_PATH")"
    export METRUM_GAMEPLAY_BENCHMARK_MODE="$GAMEPLAY_ROAD_PROFILE_MODE"
    export METRUM_GAMEPLAY_BENCHMARK_RUN_ID="$GAMEPLAY_RUN_ID"
    export METRUM_GAMEPLAY_BENCHMARK_WORLD_PATH="$GAMEPLAY_WORLD_PATH"
    export METRUM_GAMEPLAY_BENCHMARK_METRICS_PATH="$GAMEPLAY_METRICS_PATH"
    export METRUM_GAMEPLAY_BENCHMARK_PROFILED="$((GAMEPLAY_ROAD_USE_PROFILER || GAMEPLAY_ROAD_GPU_PROFILE))"
    export METRUM_GAMEPLAY_BENCHMARK_GPU_PROFILED="$GAMEPLAY_ROAD_GPU_PROFILE"
    export METRUM_GAMEPLAY_BENCHMARK_GIT_REVISION="$(git -C "$PROJECT_ROOT" rev-parse HEAD)"
    export METRUM_GAMEPLAY_BENCHMARK_GIT_DIRTY="$(git -C "$PROJECT_ROOT" status --porcelain --untracked-files=no | wc -l)"
    if ! GAMEPLAY_DIFF_SHA256="$(set -o pipefail; git -C "$PROJECT_ROOT" diff --binary HEAD | hash_stdin)"; then
        echo "Error: could not hash the benchmark working-tree diff." >&2
        exit 1
    fi
    export METRUM_GAMEPLAY_BENCHMARK_DIFF_SHA256="$GAMEPLAY_DIFF_SHA256"
    # Never let an old success document authorize this run, or overwrite a saved capture.
    if [ -e "$GAMEPLAY_METRICS_PATH" ] || [ -e "$GAMEPLAY_LOG_PATH" ] || \
       { [ "$GAMEPLAY_ROAD_USE_PROFILER" -eq 1 ] && [ -e "$GAMEPLAY_PROFILE_PATH" ]; }; then
        echo "Error: benchmark output already exists; choose a new METRUM_GAMEPLAY_BENCHMARK_RUN_ID." >&2
        exit 2
    fi

    echo "Running deterministic $GAMEPLAY_ROAD_PROFILE_MODE gameplay road workload (cpu_profile=$GAMEPLAY_ROAD_USE_PROFILER gpu_profile=$GAMEPLAY_ROAD_GPU_PROFILE)..."
    echo "  Matrix:  ${METRUM_GAMEPLAY_BENCHMARK_MATRIX:-paired}"
    if [ "$GAMEPLAY_ROAD_USE_PROFILER" -eq 1 ]; then
        echo "  Profile: $GAMEPLAY_PROFILE_PATH"
    fi
    echo "  Metrics: $GAMEPLAY_METRICS_PATH"
    echo "  Log:     $GAMEPLAY_LOG_PATH"
    cd "$GODOT_DIR"
    GAMEPLAY_COMMAND=(godot --path "$GODOT_DIR" --log-file "$GAMEPLAY_LOG_PATH")
    if [ "$GAMEPLAY_ROAD_PROFILE_MODE" = "headless" ]; then
        GAMEPLAY_COMMAND+=(--headless)
    else
        GAMEPLAY_COMMAND+=(--windowed --resolution 1920x1080)
    fi
    if [ "$GAMEPLAY_ROAD_GPU_PROFILE" -eq 1 ]; then
        GAMEPLAY_COMMAND+=(--gpu-profile)
    fi
    GAMEPLAY_COMMAND+=(-- --gameplay-road-benchmark)
    if [ "$GAMEPLAY_ROAD_USE_PROFILER" -eq 1 ]; then
        samply record --rate "$GAMEPLAY_SAMPLE_RATE" --save-only --unstable-presymbolicate \
            --profile-name "Metrum Rise gameplay roads ($GAMEPLAY_ROAD_PROFILE_MODE)" \
            --output "$GAMEPLAY_PROFILE_PATH" "${GAMEPLAY_COMMAND[@]}"
    else
        "${GAMEPLAY_COMMAND[@]}"
    fi
    PROFILE_STATUS=$?
    if [ $PROFILE_STATUS -eq 0 ] && [ "$GAMEPLAY_ROAD_USE_PROFILER" -eq 1 ]; then
        if [ ! -s "$GAMEPLAY_PROFILE_PATH" ] || ! gzip -t "$GAMEPLAY_PROFILE_PATH" 2>/dev/null; then
            echo "Error: Samply did not produce a readable profile at $GAMEPLAY_PROFILE_PATH" >&2
            PROFILE_STATUS=1
        elif [ ! -s "$GAMEPLAY_SYMBOLS_PATH" ]; then
            echo "Error: Samply did not produce symbol data at $GAMEPLAY_SYMBOLS_PATH" >&2
            PROFILE_STATUS=1
        fi
    fi
    if [ "${METRUM_GAMEPLAY_BENCHMARK_MATRIX:-paired}" = "terrain" ]; then
        if [ -s "$GAMEPLAY_METRICS_PATH" ]; then
            python3 "$PROJECT_ROOT/tools/road_terrain_replay.py" --report "$GAMEPLAY_METRICS_PATH" \
                --output "$GAMEPLAY_BASE.terrain-audit.json"
            TERRAIN_AUDIT_STATUS=$?
            if [ $PROFILE_STATUS -eq 0 ]; then
                PROFILE_STATUS=$TERRAIN_AUDIT_STATUS
            fi
        else
            echo "Error: terrain replay produced no capture." >&2
            PROFILE_STATUS=1
        fi
    elif [ $PROFILE_STATUS -eq 0 ]; then
        if ! python3 "$PROJECT_ROOT/tools/road_benchmark_report.py" --validate "$GAMEPLAY_METRICS_PATH"; then
            echo "Error: gameplay benchmark did not produce a complete valid capture." >&2
            PROFILE_STATUS=1
        fi
    fi
    echo "Gameplay road benchmark finished with status $PROFILE_STATUS."
    exit $PROFILE_STATUS
fi

if [ $ROAD_CHUNK_UPLOAD_BENCHMARK -eq 1 ]; then
    echo "Running deterministic Godot road-chunk upload benchmark..."
    godot --headless --path "$GODOT_DIR" --script res://tests/road_chunk_renderer_benchmark.gd
    exit $?
fi

if [ $ROAD_CHUNK_BENCHMARK -eq 1 ]; then
    echo "Running deterministic Rust road-chunk benchmark..."
    if ! cargo bench --bench road_chunk_benchmark; then
        echo "Rust road-chunk benchmark failed!"
        exit 1
    fi
    echo "Running deterministic Godot road-chunk upload benchmark..."
    godot --headless --path "$GODOT_DIR" --script res://tests/road_chunk_renderer_benchmark.gd
    exit $?
fi

if [ $TEST -eq 1 ]; then
    if ! python3 -m unittest discover -s "$PROJECT_ROOT/tools" -p test_road_terrain_replay.py; then
        exit 1
    fi
    if ! python3 -m unittest discover -s "$PROJECT_ROOT/tools" -p test_road_benchmark_report.py; then
        exit 1
    fi
    echo "Checking benchmark targets for API drift..."
    if ! cargo check --benches; then
        exit 1
    fi
    echo "Running Rust tests..."
    if ! cargo test; then
        echo "Rust tests failed!"
        exit 1
    fi
    echo "Running Godot bridge tests..."
    cd ../godot
    for bridge_test_script in \
        economy_machinery_test field_edit_tool_test building_asset_reload_test \
        environment_overlay_test surface_patch_debug_test simulation_speed_input_test \
        vehicle_ground_support_test selection_gesture_test ui_settings_test \
        network_tool_chunk_renderer_test road_benchmark_metrics_test \
        road_junction_preview_test road_preview_stream_test vegetation_invalidation_test \
        vegetation_edit_test new_game_dialog_test day_cycle_test camera_save_load_test \
        zoning_road_tool_test; do
        bridge_test_command=(godot --headless --script "res://tests/${bridge_test_script}.gd")
        # This regression probes inputs that previously stalled the simulation thread.
        if [ "$bridge_test_script" = simulation_speed_input_test ]; then
            if ! run_with_test_timeout "${bridge_test_command[@]}"; then
                exit 1
            fi
            continue
        fi
        if ! "${bridge_test_command[@]}"; then
            exit 1
        fi
    done
    asset_test_profile=$(mktemp -d -t metrum-asset-tests.XXXXXX) || exit 1
    asset_test_project="$asset_test_profile/project"
    mkdir "$asset_test_project" || exit 1
    # A unique project identity also isolates user:// on macOS, which ignores XDG.
    # Reuse imported resources without copying assets or changing the real project.
    for asset_test_entry in "$GODOT_DIR"/* "$GODOT_DIR/.godot"; do
        case "${asset_test_entry##*/}" in project.godot|override.cfg) continue ;; esac
        [ -e "$asset_test_entry" ] || continue
        ln -s "$asset_test_entry" "$asset_test_project/" || exit 1
    done
    cp "$GODOT_DIR/project.godot" "$asset_test_project/project.godot" || exit 1
    printf '[application]\nconfig/name="MetrumAssetTests-%s"\nconfig/use_custom_user_dir=false\n' \
        "${asset_test_profile##*/}" > "$asset_test_project/override.cfg" || exit 1
    echo "Asset regression fixtures and logs: $asset_test_profile"
    if [ "$METRUM_PLATFORM" = "darwin" ]; then
        echo "macOS fixtures: ~/Library/Application Support/Godot/app_userdata/MetrumAssetTests-${asset_test_profile##*/}"
    fi
    for asset_test_script in asset_document_test asset_authoring_test asset_workspace_test asset_layout_test asset_selection_test asset_editor_preview_test; do
        if ! XDG_DATA_HOME="$asset_test_profile/data" XDG_CONFIG_HOME="$asset_test_profile/config" \
            godot --headless --path "$asset_test_project" --script "res://tests/${asset_test_script}.gd" \
            --log-file "$asset_test_profile/${asset_test_script}.log" -- --asset-editor; then
            exit 1
        fi
        if grep -Eq '^(ERROR:|SCRIPT ERROR:)' "$asset_test_profile/${asset_test_script}.log"; then
            echo "Asset regression reported an engine/script error: $asset_test_script"
            exit 1
        fi
    done
    if [ "$METRUM_PLATFORM" = "linux" ] && command -v xvfb-run >/dev/null 2>&1; then
        echo "Running rendered terrain shader tests..."
        if ! xvfb-run -a godot --display-driver x11 --rendering-method gl_compatibility \
            --audio-driver Dummy --resolution 128x128 --script res://tests/terrain_overlay_shader_test.gd; then
            exit 1
        fi
    elif [ "$METRUM_PLATFORM" = "linux" ]; then
        echo "Rendered terrain shader tests skipped: xvfb-run is unavailable."
    else
        echo "Rendered terrain shader tests skipped on macOS: the Xvfb/X11 path is Linux-only and a direct macOS renderer path is not yet validated."
    fi
    exit 0
fi

echo "Launching Metrum Rise..."
cd ../godot && godot "${GODOT_ENGINE_ARGS[@]}" -- "${GODOT_ARGS[@]}"
