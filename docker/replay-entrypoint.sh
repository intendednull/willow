#!/bin/sh
set -e
mkdir -p /etc/willow

# Generate identity if needed.
if [ ! -f /etc/willow/replay.key ]; then
  willow-replay --generate-identity --identity-path /etc/willow/replay.key
fi

# Wait for relay peer ID to be published.
echo "Waiting for relay peer ID..."
while [ ! -f /shared/relay-peer-id ]; do sleep 1; done
RELAY_PEER_ID=$(cat /shared/relay-peer-id)
echo "Relay peer ID: $RELAY_PEER_ID"

RELAY_ADDR="/dns4/relay/tcp/9091/ws/p2p/$RELAY_PEER_ID"

exec willow-replay \
  --identity-path /etc/willow/replay.key \
  --relay "$RELAY_ADDR" \
  --max-events-per-server "${MAX_EVENTS_PER_SERVER:-1000}" \
  --sync-interval "${SYNC_INTERVAL:-30}"
