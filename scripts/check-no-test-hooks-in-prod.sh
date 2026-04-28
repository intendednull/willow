#!/usr/bin/env bash
# Asserts that a default `trunk build --release` does NOT include the
# WillowTestHooks symbol. Run as part of `just check-all` to catch
# accidental `default = ["test-hooks"]` regressions.
#
# Per docs/specs/2026-04-27-event-based-waits-design.md: the test-hooks
# feature must remain off in production for privacy reasons (third-party
# JS in prod could otherwise read DAG heads).

set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Building release (no features) ..."
(cd crates/web && trunk build --release --offline 2>&1) > /dev/null

DIST=crates/web/dist
if grep -q "WillowTestHooks" "$DIST"/*.js 2>/dev/null; then
    echo "FAIL: WillowTestHooks symbol leaked into prod JS shim:"
    grep -l "WillowTestHooks" "$DIST"/*.js
    exit 1
fi

# Defence-in-depth: check the wasm name section if wasm-objdump is
# available. If not present, skip (CI image may install on demand).
if command -v wasm-objdump >/dev/null 2>&1; then
    if wasm-objdump --section=name "$DIST"/*.wasm 2>/dev/null | grep -q "WillowTestHooks"; then
        echo "FAIL: WillowTestHooks symbol leaked into prod wasm name section"
        exit 1
    fi
fi

echo "==> Building with --features test-hooks (sanity check) ..."
(cd crates/web && trunk build --release --features test-hooks --offline 2>&1) > /dev/null

if ! grep -q "WillowTestHooks" "$DIST"/*.js 2>/dev/null; then
    echo "FAIL: WillowTestHooks symbol absent from feature build — gating broken?"
    exit 1
fi

echo "PASS: test-hooks gating verified."
