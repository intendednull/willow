#!/bin/sh
set -e
mkdir -p /etc/willow /var/lib/willow

# Generate identity if needed.
if [ ! -f /etc/willow/storage.key ]; then
  willow-storage --generate-identity --identity-path /etc/willow/storage.key
fi

RELAY_URL="${RELAY_URL:-http://relay:3340}"

exec willow-storage \
  --identity-path /etc/willow/storage.key \
  --relay-url "$RELAY_URL" \
  --db-path "${DB_PATH:-/var/lib/willow/storage.db}" \
  --sync-interval "${SYNC_INTERVAL:-60}"
