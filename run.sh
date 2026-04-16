#!/bin/bash
# Metrum Rise Run Script
#
# Debug modes:
#   --debug              General debug logging (stdout)
#   --debug <category>   Category-filtered debug logging (stdout)
#                        Common categories: isect, economy, demand, road, border
#   --debug traffic      Traffic/routing + road-network connectivity (stderr)
#   --debug-traffic      Alias for --debug traffic
#                        Shows per-road-placement split details, CCH rebuild connectivity
#                        reports, and agent routing decisions.
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

if [ $DEBUG -eq 1 ]; then
    export METRUM_DEBUG=1
    if [ -n "$DEBUG_CATEGORY" ]; then
        export METRUM_DEBUG_FILTER="$DEBUG_CATEGORY"
        echo "Debug logging enabled for category '$DEBUG_CATEGORY' (output goes to stdout)"
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
    cargo build --release
    LIB=target/release/libmetrum_rise.so
else
    cargo build
    LIB=target/debug/libmetrum_rise.so
fi
if [ $? -ne 0 ]; then
    echo "Rust build failed!"
    exit 1
fi

echo "Deploying library..."
mkdir -p ../godot/bin
cp $LIB ../godot/bin/libmetrum_rise.so

echo "Launching Metrum Rise..."
cd ../godot && godot -- "${GODOT_ARGS[@]}"
