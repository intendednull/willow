# Willow — P2P encrypted chat

# Run ALL checks (fmt, clippy, test, wasm, browser). Use before committing.
check: fmt clippy test check-wasm

# Format all code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt --check

# Run clippy with warnings as errors
clippy:
    cargo clippy --workspace -- -D warnings

# Run all cargo tests (unit + integration, excludes browser)
test:
    cargo test --workspace

# Run tests for a specific crate
test-crate crate:
    cargo test -p {{crate}}

# Run the state machine tests
test-state:
    cargo test -p willow-state

# Run the client library tests
test-client:
    cargo test -p willow-client

# Run the Bevy app headless + integration tests
test-app:
    cargo test -p willow-app

# Run the relay history sync tests
test-relay:
    cargo test -p willow-relay

# Run the scaling / performance tests with output
test-scale:
    cargo test -p willow-app --test peer_scale -- --nocapture

# Run the end-to-end flow integration tests
test-e2e:
    cargo test -p willow-app --test e2e_flow -- --nocapture

# Run in-browser Leptos component tests (requires Firefox + geckodriver)
test-browser:
    wasm-pack test --headless --firefox crates/web

# Run ALL tests including browser tests
test-all: test test-browser

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

# Build the Leptos web app (WASM)
build-web:
    cd crates/web && trunk build --release

# Serve the Leptos web app locally
serve-web:
    cd crates/web && trunk serve

# Build the legacy Bevy WASM app
build-wasm:
    cargo build --release --target wasm32-unknown-unknown -p willow-app
    wasm-bindgen \
        --out-dir web/pkg \
        --target web \
        --no-typescript \
        target/wasm32-unknown-unknown/release/willow-app.wasm

# Build and serve the legacy Bevy WASM app on localhost:8080
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
    rm -rf web/pkg crates/web/dist
