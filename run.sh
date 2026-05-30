#!/bin/bash
# Metrum Rise Run Script
#
# Debug modes:
#   --debug              General debug logging (stdout)
#   --debug <category>   Category-filtered debug logging (stdout)
#                        Common categories: isect, economy, demand, road, border, terrain
#   --debug road         Road placement timings, committed-road geometry dumps,
#                        terrain/water patch diagnostics, and road-surface overlay
#   --debug terrain      Terrain + water patch residency/perf summaries (stdout)
#                        Shows resident patch counts, desired bounds, cull distance, patch
#                        create/remove/upload churn, and average renderer timings while flying.
#   --debug terrain-verbose
#                        Same as terrain plus residency-change logs whenever the patch window moves.
#   --debug terrain-full
#                        Same as terrain plus forced full-world terrain/water residency to compare
#                        steady-state full-map cost against camera-driven patch churn.
#   --debug traffic      Traffic/routing + road-network connectivity (stderr)
#   --debug-traffic      Alias for --debug traffic
#                        Shows per-road-placement split details, CCH rebuild connectivity
#                        reports, and agent routing decisions.
#   --debug-world-editor Alias for --debug world-editor
#                        Shows world-editor create/open/save/tool activity (stdout)
#   --debug-sim          Hourly simulation summaries (stdout)

RELEASE=0
DEBUG=0
DEBUG_TRAFFIC=0
DEBUG_SIM=0
DEBUG_CATEGORY=""
GODOT_ARGS=()
export RUST_BACKTRACE=1
i=1
while [ $i -le $# ]; do
    arg="${!i}"
    if [ "$arg" = "--release" ]; then
        RELEASE=1
    elif [ "$arg" = "--debug-sim" ]; then
        DEBUG_SIM=1
    elif [ "$arg" = "--debug-traffic" ]; then
        DEBUG_TRAFFIC=1
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

if [ "$DEBUG_CATEGORY" = "road-geometry" ]; then
    echo "Error: --debug road-geometry was removed. Use --debug road." >&2
    exit 2
fi

if [ $DEBUG -eq 1 ]; then
    export METRUM_DEBUG=1
    if [ -n "$DEBUG_CATEGORY" ]; then
        if [ "$DEBUG_CATEGORY" = "road" ]; then
            export METRUM_DEBUG_FILTER="road"
            export METRUM_DEBUG_ROAD_GEOMETRY_DUMP=1
            export METRUM_DEBUG_SURFACE=1
        else
            export METRUM_DEBUG_FILTER="$DEBUG_CATEGORY"
        fi
        echo "Debug logging enabled for category '$DEBUG_CATEGORY' (output goes to stdout)"
        if [ "$DEBUG_CATEGORY" = "road" ]; then
            echo "  Road placement timing summaries enabled."
            echo "  After each committed road refresh: [DEBUG:road] ROAD_GEOMETRY_DUMP_BEGIN ... ROAD_GEOMETRY_DUMP_END"
            echo "  Includes edge geometry, node class/throat diagnostics, compiled loops, and source/visual terrain samples."
            echo "  Also prints terrain/water patch clip, mesh, baseline-vs-dynamic water diagnostics, and authored fill contributors for road-touched patches."
            echo "  Also enables the compiled road-surface overlay in the editor for visual comparison."
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
fi
if [ $DEBUG_SIM -eq 1 ]; then
    export METRUM_DEBUG_SIM=1
    echo "Simulation console debug enabled (hourly summaries go to stdout)"
fi

echo "Building Rust library..."
cd rust
if [ $RELEASE -eq 1 ]; then
    if ! cargo build --release; then
        echo "Rust build failed!"
        exit 1
    fi
    LIB=target/release/libmetrum_rise.so
else
    if ! cargo build; then
        echo "Rust build failed!"
        exit 1
    fi
    LIB=target/debug/libmetrum_rise.so
fi

echo "Deploying library..."
mkdir -p ../godot/bin
cp $LIB ../godot/bin/libmetrum_rise.so

echo "Launching Metrum Rise..."
cd ../godot && godot -- "${GODOT_ARGS[@]}"
