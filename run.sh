#!/bin/bash
# Metrum Rise Run Script

RELEASE=0
GODOT_ARGS=()
export RUST_BACKTRACE=1
for arg in "$@"; do
    if [ "$arg" = "--release" ]; then
        RELEASE=1
    else
        GODOT_ARGS+=("$arg")
    fi
done

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
