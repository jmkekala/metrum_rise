#!/bin/bash
# Metrum Rise Run Script

echo "Building Rust library..."
cd rust && cargo build
if [ $? -ne 0 ]; then
    echo "Rust build failed!"
    exit 1
fi

echo "Deploying library..."
mkdir -p ../godot/bin
cp target/debug/libmetrum_rise.so ../godot/bin/libmetrum_rise.so

echo "Launching Metrum Rise..."
cd ../godot && godot
