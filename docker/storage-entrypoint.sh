#!/bin/sh
set -e
mkdir -p /etc/willow /var/lib/willow

# Generate identity if needed.
if [ ! -f /etc/willow/storage.key ]; then
  willow-storage --generate-identity --identity-path /etc/willow/storage.key
fi

# Wait for relay peer ID to be published.
echo "Waiting for relay peer ID..."
while [ ! -f /shared/relay-peer-id ]; do sleep 1; done
RELAY_PEER_ID=$(cat /shared/relay-peer-id)
echo "Relay peer ID: $RELAY_PEER_ID"

RELAY_ADDR="/dns4/relay/tcp/9091/ws/p2p/$RELAY_PEER_ID"

exec willow-storage \
  --identity-path /etc/willow/storage.key \
  --relay "$RELAY_ADDR" \
  --db-path "${DB_PATH:-/var/lib/willow/storage.db}" \
  --sync-interval "${SYNC_INTERVAL:-60}"
