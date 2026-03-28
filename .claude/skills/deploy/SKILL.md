---
name: deploy
description: Build and deploy the Willow relay, worker nodes, and web UI to the Linode server
user-invocable: true
---

# Deploy Willow

Build and deploy the full Willow stack: relay server, worker nodes
(replay + storage), and Leptos web app.

## Server Details

- **IP**: 172.234.217.219
- **SSH**: `sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219`
- **Web app**: http://172.234.217.219 (nginx serving /var/www/willow/)
- **Relay**: systemd service `willow-relay` on ports 9090 (TCP) and 9091 (WebSocket)
- **Replay**: systemd service `willow-replay` (connects to relay as a peer)
- **Storage**: systemd service `willow-storage` (connects to relay as a peer)

## Architecture

```
┌──────────────────────────────────────┐
│  Production Server (172.234.217.219) │
│                                      │
│  nginx         → /var/www/willow/    │
│  willow-relay  → ports 9090/9091     │
│  willow-replay → peer (in-memory)    │
│  willow-storage→ peer (SQLite)       │
│                                      │
└──────────────────────────────────────┘
```

The relay is stateless network plumbing (TCP↔WS bridging, NAT traversal).
Worker nodes handle state persistence:
- **Replay node**: fast bounded-memory state sync for peers coming online
- **Storage node**: archival SQLite-backed history for paginated queries

## Steps

### 1. Build all binaries (if changed)

```bash
cargo build --release -p willow-relay -p willow-replay -p willow-storage
```

### 2. Build the Leptos web app

```bash
cd crates/web && trunk build --release && cd ../..
```

### 3. Upload to server

```bash
# Upload binaries (only if rebuilt)
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'systemctl stop willow-replay willow-storage willow-relay 2>/dev/null; true'

sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no \
    target/release/willow-relay \
    target/release/willow-replay \
    target/release/willow-storage \
    root@172.234.217.219:/usr/local/bin/

# Upload web app
sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no crates/web/dist/* root@172.234.217.219:/var/www/willow/

# Fix permissions
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'chmod 755 /usr/local/bin/willow-relay /usr/local/bin/willow-replay /usr/local/bin/willow-storage && chmod 644 /var/www/willow/*'
```

### 4. Set up worker systemd services (first deploy only)

```bash
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'bash -s' << 'SETUP'
# Create data directories
mkdir -p /etc/willow /var/lib/willow

# Generate worker identities if they don't exist
if [ ! -f /etc/willow/replay.key ]; then
    /usr/local/bin/willow-replay --generate-identity --identity-path /etc/willow/replay.key
fi
if [ ! -f /etc/willow/storage.key ]; then
    /usr/local/bin/willow-storage --generate-identity --identity-path /etc/willow/storage.key
fi

# Get the relay's peer ID for worker config
RELAY_PEER_ID=$(/usr/local/bin/willow-relay --identity /etc/willow/relay.key 2>&1 | head -1 || true)
# If relay key doesn't exist yet, it'll be created on first start

# Create replay service
cat > /etc/systemd/system/willow-replay.service << EOF
[Unit]
Description=Willow Replay Worker Node
After=willow-relay.service
Requires=willow-relay.service

[Service]
Type=simple
ExecStart=/usr/local/bin/willow-replay \
    --identity-path /etc/willow/replay.key \
    --relay /ip4/127.0.0.1/tcp/9091/ws/p2p/RELAY_PEER_ID_PLACEHOLDER \
    --max-events-per-server 1000 \
    --sync-interval 30
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

# Create storage service
cat > /etc/systemd/system/willow-storage.service << EOF
[Unit]
Description=Willow Storage Worker Node
After=willow-relay.service
Requires=willow-relay.service

[Service]
Type=simple
ExecStart=/usr/local/bin/willow-storage \
    --identity-path /etc/willow/storage.key \
    --relay /ip4/127.0.0.1/tcp/9091/ws/p2p/RELAY_PEER_ID_PLACEHOLDER \
    --db-path /var/lib/willow/storage.db \
    --sync-interval 60
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
SETUP
```

**After creating services**, you must replace `RELAY_PEER_ID_PLACEHOLDER`
in both service files with the actual relay peer ID. Get it by running:

```bash
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'systemctl start willow-relay && sleep 2 && journalctl -u willow-relay -n 5 | grep peer_id'
```

Then update both service files and reload:

```bash
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    "sed -i 's/RELAY_PEER_ID_PLACEHOLDER/ACTUAL_PEER_ID/' /etc/systemd/system/willow-replay.service /etc/systemd/system/willow-storage.service && systemctl daemon-reload"
```

### 5. Restart services

```bash
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'systemctl restart willow-relay && sleep 2 && systemctl restart willow-replay willow-storage'
```

### 6. Enable on boot (first deploy only)

```bash
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'systemctl enable willow-replay willow-storage'
```

### 7. Verify

```bash
# Check web app
curl -s -o /dev/null -w "Web: HTTP %{http_code}\n" http://172.234.217.219/

# Check all services
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'echo "Relay: $(systemctl is-active willow-relay)"; echo "Replay: $(systemctl is-active willow-replay)"; echo "Storage: $(systemctl is-active willow-storage)"'

# Print worker peer IDs (needed for PLATFORM_WORKERS)
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'echo "Replay PeerID:"; /usr/local/bin/willow-replay --print-peer-id --identity-path /etc/willow/replay.key; echo "Storage PeerID:"; /usr/local/bin/willow-storage --print-peer-id --identity-path /etc/willow/storage.key'
```

## Quick deploy (web only)

For CSS/component changes that don't touch binaries:

```bash
cd crates/web && trunk build --release && cd ../.. && sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no crates/web/dist/* root@172.234.217.219:/var/www/willow/ && sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'chmod 644 /var/www/willow/*'
```

## Quick deploy (relay + workers only)

For backend changes that don't touch the web UI:

```bash
cargo build --release -p willow-relay -p willow-replay -p willow-storage && \
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'systemctl stop willow-replay willow-storage willow-relay' && \
sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no target/release/willow-relay target/release/willow-replay target/release/willow-storage root@172.234.217.219:/usr/local/bin/ && \
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'chmod 755 /usr/local/bin/willow-relay /usr/local/bin/willow-replay /usr/local/bin/willow-storage && systemctl restart willow-relay && sleep 2 && systemctl restart willow-replay willow-storage'
```

## Scaling workers

To add more worker instances, create additional systemd services with
unique identity paths:

```bash
# Generate new identity
willow-replay --generate-identity --identity-path /etc/willow/replay-2.key

# Copy and modify the service file
cp /etc/systemd/system/willow-replay.service /etc/systemd/system/willow-replay-2.service
# Edit to use replay-2.key
systemctl daemon-reload
systemctl enable --now willow-replay-2
```

## Troubleshooting

```bash
# View logs
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'journalctl -u willow-relay -n 20 --no-pager'
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'journalctl -u willow-replay -n 20 --no-pager'
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'journalctl -u willow-storage -n 20 --no-pager'

# Check if workers can reach the relay
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 \
    'ss -tlnp | grep -E "9090|9091"'
```
