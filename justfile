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

# Run the relay tests
test-relay:
    cargo test -p willow-relay

# Run worker node tests (worker library + replay + storage)
test-workers:
    cargo test -p willow-worker -p willow-replay -p willow-storage -p willow-common

# Build worker node binaries
build-workers:
    cargo build --release -p willow-replay -p willow-storage

# Run the scaling / performance tests with output
test-scale:
    cargo test -p willow-app --test peer_scale -- --nocapture

# Run the end-to-end flow integration tests
test-e2e:
    cargo test -p willow-app --test e2e_flow -- --nocapture

# Run in-browser Leptos component tests (requires Firefox + geckodriver)
test-browser:
    wasm-pack test --headless --firefox crates/web

# Run Playwright E2E tests against deployed site
test-e2e-ui:
    npx playwright test --project=desktop-chrome --project=mobile-chrome

# Run Playwright E2E tests on all browsers
test-e2e-ui-all:
    npx playwright test

# Run Playwright E2E tests (headed, for debugging)
test-e2e-ui-headed:
    npx playwright test --headed

# Run multi-peer sync tests (desktop-chrome for quick iteration)
test-e2e-sync:
    npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome

# Run permission tests
test-e2e-perms:
    npx playwright test e2e/permissions.spec.ts --project=desktop-chrome

# Run ALL tests including browser and E2E
test-all: test test-browser test-e2e-ui

# Check native compilation
check-native:
    cargo check

# Check WASM compilation (excludes native-only binaries)
check-wasm:
    cargo check --target wasm32-unknown-unknown --workspace --exclude willow-relay --exclude willow-worker --exclude willow-replay --exclude willow-storage

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

# Docker: build all images
docker-build:
    docker compose build

# Docker: start full stack
docker-up:
    docker compose up -d

# Docker: stop full stack
docker-down:
    docker compose down

# Docker: tail all logs
docker-logs:
    docker compose logs -f

# Docker: print all worker peer IDs
docker-ids:
    @docker compose exec replay-1 willow-replay --print-peer-id 2>/dev/null || echo "replay-1: not running"
    @docker compose exec replay-2 willow-replay --print-peer-id 2>/dev/null || echo "replay-2: not running"
    @docker compose exec storage-1 willow-storage --print-peer-id 2>/dev/null || echo "storage-1: not running"
    @docker compose exec storage-2 willow-storage --print-peer-id 2>/dev/null || echo "storage-2: not running"

# Start all services for local development (relay, workers, web UI)
dev:
    ./scripts/dev.sh

# Start all services, skipping the build step
dev-quick:
    ./scripts/dev.sh --skip-build

# Clean dev data (identity keys, logs, storage DB)
dev-clean:
    rm -rf .dev

# Clean build artifacts
clean:
    cargo clean
    rm -rf web/pkg crates/web/dist
