#!/bin/sh
set -e
mkdir -p /etc/willow /shared

# Generate key and publish peer ID for workers.
PEER_ID=$(willow-relay --identity /etc/willow/relay.key --print-peer-id)
echo "$PEER_ID" > /shared/relay-peer-id
echo "Relay peer ID: $PEER_ID"

exec willow-relay --tcp-port 9090 --ws-port 9091 --identity /etc/willow/relay.key
