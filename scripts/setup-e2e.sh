#!/usr/bin/env bash
# scripts/setup-e2e.sh — Prepare the E2E test environment from scratch.
#
# Installs tooling, builds all services, starts the full dev stack,
# and waits until everything is ready for Playwright tests.
#
# Usage:
#   ./scripts/setup-e2e.sh          # full setup + start services
#   ./scripts/setup-e2e.sh --no-start  # install/build only, don't start services

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEV_DIR="$ROOT/.dev"
LOG_DIR="$DEV_DIR/logs"

NO_START=false
for arg in "$@"; do
    case "$arg" in
        --no-start) NO_START=true ;;
    esac
done

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

step() { echo -e "${GREEN}[setup]${NC} $1"; }
info() { echo -e "${BLUE}[info]${NC} $1"; }
fail() { echo -e "${RED}[error]${NC} $1"; exit 1; }

# ── 1. Install tooling ──────────────────────────────────────────────────

step "Installing tooling..."

# WASM target
if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
    step "Adding wasm32-unknown-unknown target..."
    rustup target add wasm32-unknown-unknown
fi

# trunk
if ! command -v trunk &>/dev/null; then
    step "Installing trunk (WASM bundler)..."
    cargo install --locked --version 0.21.14 trunk 2>&1 | tail -1
fi

# just
if ! command -v just &>/dev/null; then
    step "Installing just (task runner)..."
    cargo install --locked --version 1.50.0 just 2>&1 | tail -1
fi

# npm dependencies
if [ ! -d "$ROOT/node_modules" ]; then
    step "Installing npm dependencies..."
    (cd "$ROOT" && npm ci)
fi

# Playwright browsers. `--dry-run` prints the install location whether
# or not the browser is present, so check the filesystem instead. Skip
# `--with-deps` — it triggers a non-interactive sudo prompt that fails
# in sandboxed dev shells; assume OS packages are already present or
# install them once out-of-band.
if ! ls "$HOME/.cache/ms-playwright" 2>/dev/null | grep -q '^chromium-'; then
    step "Installing Playwright Chromium..."
    npx playwright install chromium
fi

info "Tooling ready: trunk=$(trunk --version 2>/dev/null || echo missing), just=$(just --version 2>/dev/null || echo missing)"

# ── 2. Build all services ───────────────────────────────────────────────

step "Building relay, replay, and storage..."
cargo build -p willow-relay -p willow-replay -p willow-storage 2>&1 | tail -1

FEATURES="${WILLOW_FEATURES:-}"
FEATURES_FLAG=""
if [ -n "$FEATURES" ]; then
    FEATURES_FLAG="--features $FEATURES"
fi

step "Building web UI (WASM)..."
# shellcheck disable=SC2086
(cd "$ROOT/crates/web" && trunk build $FEATURES_FLAG 2>&1 | tail -1)

info "All builds complete."

if [ "$NO_START" = true ]; then
    info "Skipping service startup (--no-start)."
    exit 0
fi

# ── 3. Start services ───────────────────────────────────────────────────

mkdir -p "$DEV_DIR" "$LOG_DIR"

# Kill any leftover processes from a previous run.
pkill -f "willow-relay" 2>/dev/null || true
pkill -f "willow-replay" 2>/dev/null || true
pkill -f "willow-storage" 2>/dev/null || true
pkill -f "trunk serve" 2>/dev/null || true
sleep 1

# Relay
step "Starting relay..."
cargo run -p willow-relay -- \
    --relay-port 3340 \
    --identity "$DEV_DIR/relay.key" \
    > "$LOG_DIR/relay.log" 2>&1 &
RELAY_PID=$!

# Wait for relay to be ready
RELAY_READY=false
for i in $(seq 1 60); do
    if grep -q "relay running" "$LOG_DIR/relay.log" 2>/dev/null; then
        RELAY_READY=true
        break
    fi
    sleep 1
done
if [ "$RELAY_READY" = false ]; then
    fail "Relay failed to start. Check $LOG_DIR/relay.log"
fi
info "Relay running on port 3340 (PID $RELAY_PID)"

RELAY_URL="http://127.0.0.1:3340"

# Replay node
step "Starting replay node..."
cargo run -p willow-replay -- \
    --identity-path "$DEV_DIR/replay.key" \
    --relay-url "$RELAY_URL" \
    --max-events-per-author 1000 \
    --sync-interval 10 \
    > "$LOG_DIR/replay.log" 2>&1 &
info "Replay node started (PID $!)"

# Storage node
step "Starting storage node..."
cargo run -p willow-storage -- \
    --identity-path "$DEV_DIR/storage.key" \
    --relay-url "$RELAY_URL" \
    --db-path "$DEV_DIR/storage.db" \
    --sync-interval 15 \
    > "$LOG_DIR/storage.log" 2>&1 &
info "Storage node started (PID $!)"

# Web UI
step "Starting web UI (trunk serve)..."
# shellcheck disable=SC2086
(cd "$ROOT/crates/web" && trunk serve $FEATURES_FLAG) > "$LOG_DIR/web.log" 2>&1 &
WEB_PID=$!

# Wait for web UI
WEB_READY=false
for i in $(seq 1 180); do
    if curl -s http://127.0.0.1:8080 > /dev/null 2>&1; then
        WEB_READY=true
        break
    fi
    sleep 1
done
if [ "$WEB_READY" = false ]; then
    fail "Web UI failed to start. Check $LOG_DIR/web.log"
fi
info "Web UI running at http://127.0.0.1:8080 (PID $WEB_PID)"

# ── 4. Summary ──────────────────────────────────────────────────────────

echo ""
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo -e "${GREEN}  E2E test environment ready${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo -e "  Relay:   ${BLUE}localhost:3340${NC}"
echo -e "  Replay:  connected to relay"
echo -e "  Storage: connected to relay"
echo -e "  Web UI:  ${BLUE}http://127.0.0.1:8080${NC}"
echo -e "  Logs:    ${LOG_DIR}/"
echo -e ""
echo -e "  Run tests:"
echo -e "    npx playwright test --project=desktop-chrome"
echo -e "    just test-e2e-ui"
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
