#!/usr/bin/env bash
# scripts/dev.sh — Start all Willow services for local development.
#
# Usage:
#   ./scripts/dev.sh          # start all services
#   ./scripts/dev.sh --skip-build  # skip cargo build step
#
# Services started:
#   - Relay       (HTTP on port 3340)
#   - Replay node (in-memory, max 1000 events/author)
#   - Storage node (SQLite at .dev/storage.db)
#   - Web UI      (trunk serve on localhost:8080)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEV_DIR="$ROOT/.dev"
LOG_DIR="$DEV_DIR/logs"

# Colors for service labels
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
NC='\033[0m' # No Color

SKIP_BUILD=false
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
    esac
done

# Ensure dev directories exist
mkdir -p "$DEV_DIR" "$LOG_DIR"

# Track child PIDs for cleanup
PIDS=()

cleanup() {
    echo ""
    echo -e "${RED}Shutting down all services...${NC}"
    for pid in "${PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    wait 2>/dev/null || true
    echo -e "${GREEN}All services stopped.${NC}"
}
trap cleanup EXIT INT TERM

# Prefix each line of a command's output with a colored label
run_with_prefix() {
    local color="$1" label="$2"
    shift 2
    "$@" 2>&1 | while IFS= read -r line; do
        echo -e "${color}[${label}]${NC} $line"
    done
}

# --- Build -------------------------------------------------------------------

if [ "$SKIP_BUILD" = false ]; then
    echo -e "${GREEN}Building all services...${NC}"
    cargo build -p willow-relay -p willow-replay -p willow-storage 2>&1 | \
        while IFS= read -r line; do echo -e "${GREEN}[build]${NC} $line"; done
    echo -e "${GREEN}Build complete.${NC}"
    echo ""
fi

# --- Relay --------------------------------------------------------------------

RELAY_IDENTITY="$DEV_DIR/relay.key"
RELAY_LOG="$LOG_DIR/relay.log"

RELAY_PORT="${WILLOW_RELAY_PORT:-3340}"

echo -e "${BLUE}Starting relay...${NC}"
cargo run -p willow-relay -- \
    --relay-port "$RELAY_PORT" \
    --identity "$RELAY_IDENTITY" \
    > "$RELAY_LOG" 2>&1 &
RELAY_PID=$!
PIDS+=("$RELAY_PID")

# Wait for the relay to start (up to 30s — first run may compile)
RELAY_READY=false
for i in $(seq 1 150); do
    if grep -q "relay running" "$RELAY_LOG" 2>/dev/null; then
        RELAY_READY=true
        break
    fi
    sleep 0.2
done

if [ "$RELAY_READY" = false ]; then
    echo -e "${RED}Failed to start relay. Check $RELAY_LOG${NC}"
    exit 1
fi

RELAY_URL="http://127.0.0.1:$RELAY_PORT"
echo -e "${BLUE}Relay started:${NC} $RELAY_URL"
echo ""

# Tail relay logs with prefix
tail -f "$RELAY_LOG" 2>/dev/null | while IFS= read -r line; do
    echo -e "${BLUE}[relay]${NC} $line"
done &
PIDS+=($!)

# --- Replay node --------------------------------------------------------------

REPLAY_IDENTITY="$DEV_DIR/replay.key"
echo -e "${YELLOW}Starting replay node...${NC}"
cargo run -p willow-replay -- \
    --identity-path "$REPLAY_IDENTITY" \
    --relay-url "$RELAY_URL" \
    --max-events-per-author 1000 \
    --sync-interval 10 \
    > "$LOG_DIR/replay.log" 2>&1 &
PIDS+=($!)

tail -f "$LOG_DIR/replay.log" 2>/dev/null | while IFS= read -r line; do
    echo -e "${YELLOW}[replay]${NC} $line"
done &
PIDS+=($!)

# --- Storage node -------------------------------------------------------------

STORAGE_IDENTITY="$DEV_DIR/storage.key"
STORAGE_DB="$DEV_DIR/storage.db"
echo -e "${MAGENTA}Starting storage node...${NC}"
cargo run -p willow-storage -- \
    --identity-path "$STORAGE_IDENTITY" \
    --relay-url "$RELAY_URL" \
    --db-path "$STORAGE_DB" \
    --sync-interval 15 \
    > "$LOG_DIR/storage.log" 2>&1 &
PIDS+=($!)

tail -f "$LOG_DIR/storage.log" 2>/dev/null | while IFS= read -r line; do
    echo -e "${MAGENTA}[storage]${NC} $line"
done &
PIDS+=($!)

# --- Web UI -------------------------------------------------------------------

echo -e "${GREEN}Starting web UI (trunk serve)...${NC}"
(cd "$ROOT/crates/web" && trunk serve) 2>&1 | while IFS= read -r line; do
    echo -e "${GREEN}[web]${NC} $line"
done &
PIDS+=($!)

# --- Vibe Annotations (optional) ---------------------------------------------

CYAN='\033[0;36m'
VIBE_RUNNING=false
if command -v vibe-annotations-server &>/dev/null; then
    echo -e "${CYAN}Starting vibe-annotations server...${NC}"
    vibe-annotations-server start > "$LOG_DIR/vibe-annotations.log" 2>&1 &
    PIDS+=($!)
    VIBE_RUNNING=true
fi

# --- Summary ------------------------------------------------------------------

echo ""
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Willow dev stack running${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo -e "  Relay:   ${BLUE}$RELAY_URL${NC}"
echo -e "  Web UI:  ${GREEN}http://localhost:8080${NC}"
if [ "$VIBE_RUNNING" = true ]; then
    echo -e "  Vibe:    ${CYAN}http://127.0.0.1:3846${NC}"
fi
echo -e "  Logs:    ${LOG_DIR}/"
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo -e "  Press ${RED}Ctrl+C${NC} to stop all services"
echo ""

# Wait forever (cleanup runs on signal)
wait
