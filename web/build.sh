#!/usr/bin/env bash
# Build the Willow app for WASM deployment.
#
# Prerequisites:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli
#
# Usage:
#   ./web/build.sh          # build
#   ./web/build.sh serve    # build + serve on localhost:8080

set -euo pipefail
cd "$(dirname "$0")/.."

echo "Building willow-app for wasm32-unknown-unknown..."
cargo build --release --target wasm32-unknown-unknown -p willow-app

echo "Running wasm-bindgen..."
wasm-bindgen \
    --out-dir web/pkg \
    --target web \
    --no-typescript \
    target/wasm32-unknown-unknown/release/willow-app.wasm

echo "Build complete. Output in web/pkg/"

if [ "${1:-}" = "serve" ]; then
    echo "Serving on http://localhost:8080"
    python3 -m http.server 8080 --directory web
fi
