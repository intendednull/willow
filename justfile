# Willow — P2P encrypted chat

# Run all checks (fmt, clippy, test, wasm). Use before committing.
check: fmt clippy test check-wasm

# Format all code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt --check

# Run clippy with warnings as errors
clippy:
    cargo clippy -- -D warnings

# Run all tests
test:
    cargo test

# Run tests for a specific crate
test-crate crate:
    cargo test -p {{crate}}

# Check native compilation
check-native:
    cargo check

# Check WASM compilation (excludes native-only relay server)
check-wasm:
    cargo check --target wasm32-unknown-unknown --workspace --exclude willow-relay

# Build the native desktop app
build:
    cargo build -p willow-app

# Build the native desktop app (release)
build-release:
    cargo build --release -p willow-app

# Run the native desktop app
run:
    cargo run -p willow-app

# Build the WASM web app
build-wasm:
    cargo build --release --target wasm32-unknown-unknown -p willow-app
    wasm-bindgen \
        --out-dir web/pkg \
        --target web \
        --no-typescript \
        target/wasm32-unknown-unknown/release/willow-app.wasm

# Build and serve the WASM web app on localhost:8080
serve-wasm: build-wasm
    python3 -m http.server 8080 --directory web

# Build the relay server
build-relay:
    cargo build --release -p willow-relay

# Run the relay server (TCP 9090, WebSocket 9091)
relay *args:
    cargo run -p willow-relay -- {{args}}

# Clean build artifacts
clean:
    cargo clean
    rm -rf web/pkg
