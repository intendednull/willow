#!/bin/sh
set -e
mkdir -p /etc/willow

# Generate identity if needed.
if [ ! -f /etc/willow/replay.key ]; then
  willow-replay --generate-identity --identity-path /etc/willow/replay.key
fi

RELAY_URL="${RELAY_URL:-http://relay:3340}"

exec willow-replay \
  --identity-path /etc/willow/replay.key \
  --relay-url "$RELAY_URL" \
  --max-events-per-author "${MAX_EVENTS_PER_AUTHOR:-1000}" \
  --sync-interval "${SYNC_INTERVAL:-30}"
