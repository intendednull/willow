#!/usr/bin/env bash
# scripts/teardown-e2e.sh — Stop services started by scripts/setup-e2e.sh.
#
# Kills relay, replay, storage, and trunk serve processes started for
# E2E tests. Safe to run even if nothing is running.
#
# Usage:
#   ./scripts/teardown-e2e.sh

set -u

GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

info() { echo -e "${BLUE}[teardown]${NC} $1"; }
done_msg() { echo -e "${GREEN}[teardown]${NC} $1"; }

info "Stopping E2E services..."

# Kill processes started by setup-e2e.sh. pkill returns non-zero when
# no matching process exists — suppress so this script is idempotent.
pkill -f "willow-relay" 2>/dev/null || true
pkill -f "willow-replay" 2>/dev/null || true
pkill -f "willow-storage" 2>/dev/null || true
pkill -f "trunk serve" 2>/dev/null || true

# Give processes a moment to exit gracefully, then force-kill leftovers.
sleep 1
pkill -9 -f "willow-relay" 2>/dev/null || true
pkill -9 -f "willow-replay" 2>/dev/null || true
pkill -9 -f "willow-storage" 2>/dev/null || true
pkill -9 -f "trunk serve" 2>/dev/null || true

done_msg "E2E services stopped."
