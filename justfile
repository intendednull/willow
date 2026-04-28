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
    cargo clippy --workspace --all-targets -- -D warnings

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

# Run actor framework tests
test-actor:
    cargo test -p willow-actor

# Run actor framework performance tests (not in CI — for optimization dev)
test-actor-perf:
    cargo test -p willow-actor --test performance -- --ignored --nocapture

# Run the relay tests
test-relay:
    cargo test -p willow-relay

# Run worker node tests (worker library + replay + storage)
test-workers:
    cargo test -p willow-worker -p willow-replay -p willow-storage -p willow-common

# Build worker node binaries
build-workers:
    cargo build --release -p willow-replay -p willow-storage

# Run in-browser Leptos component tests (requires Firefox + geckodriver)
test-browser:
    wasm-pack test --headless --firefox crates/web

# Bootstrap the E2E test environment (install tooling, build, start services)
setup-e2e FEATURES="":
    WILLOW_FEATURES="{{FEATURES}}" ./scripts/setup-e2e.sh

# Install/build for E2E but don't start services
setup-e2e-no-start:
    ./scripts/setup-e2e.sh --no-start

# Stop any services started by setup-e2e
teardown-e2e:
    ./scripts/teardown-e2e.sh

# Run the full E2E flow: setup, run tests, teardown (teardown runs even on failure)
test-e2e-full FEATURES="test-hooks":
    #!/usr/bin/env bash
    set -u
    WILLOW_FEATURES="{{FEATURES}}" ./scripts/setup-e2e.sh
    EXIT_CODE=0
    npx playwright test --project=desktop-chrome --project=mobile-chrome || EXIT_CODE=$?
    ./scripts/teardown-e2e.sh
    exit $EXIT_CODE

# Run Playwright E2E tests against deployed site
test-e2e-ui FEATURES="test-hooks":
    @just setup-e2e FEATURES={{FEATURES}}
    npx playwright test --project=desktop-chrome --project=mobile-chrome

# Full-suite gate: lint + Rust + wasm-pack browser + Playwright, in
# order, fail-fast. This is the single command a PR must go green on.
check-all FEATURES="test-hooks":
    #!/usr/bin/env bash
    set -euo pipefail
    just fmt
    just clippy
    just test
    just test-browser
    just test-e2e-ui FEATURES={{FEATURES}}
    ./scripts/check-no-test-hooks-in-prod.sh

# Run Playwright E2E tests on all browsers
test-e2e-ui-all:
    npx playwright test

# Run Playwright E2E tests (headed, for debugging)
test-e2e-ui-headed:
    npx playwright test --headed

# Run multi-peer sync tests (desktop-chrome for quick iteration)
test-e2e-sync FEATURES="test-hooks":
    @just setup-e2e FEATURES={{FEATURES}}
    npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome

# Run permission tests
test-e2e-perms FEATURES="test-hooks":
    @just setup-e2e FEATURES={{FEATURES}}
    npx playwright test e2e/permissions.spec.ts --project=desktop-chrome

# Run agent unit + integration tests
test-agent:
    cargo test -p willow-agent

# Run E2E multi-peer tests via agent harness
test-agent-e2e:
    cargo test -p willow-agent --test e2e -- --nocapture

# Build the agent binary
build-agent:
    cargo build -p willow-agent

# Build agent (release)
build-agent-release:
    cargo build --release -p willow-agent

# Run the agent
agent *args:
    cargo run -p willow-agent -- {{args}}

# Run ALL tests including browser and E2E
test-all: test test-browser test-agent-e2e test-e2e-ui

# Check native compilation
check-native:
    cargo check

# Check WASM compilation (excludes native-only binaries that pull in tokio/mio)
check-wasm:
    cargo check --target wasm32-unknown-unknown -p willow-identity -p willow-state -p willow-messaging -p willow-crypto -p willow-transport -p willow-common -p willow-network -p willow-client -p willow-web

# Build the Leptos web app (WASM)
build-web:
    cd crates/web && trunk build --release

# Serve the Leptos web app locally
serve-web:
    cd crates/web && trunk serve

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
dev FEATURES="":
    WILLOW_FEATURES="{{FEATURES}}" ./scripts/dev.sh

# Start all services, skipping the build step
dev-quick:
    ./scripts/dev.sh --skip-build

# Clean dev data (identity keys, logs, storage DB)
dev-clean:
    rm -rf .dev

# Clean build artifacts
clean:
    cargo clean
    rm -rf crates/web/dist
