---
name: deploy
description: Build and deploy the Willow relay server and Leptos web UI to the Linode server
user-invocable: true
---

# Deploy Willow

Build the relay server and Leptos web app, then deploy to the production server.

## Server Details

- **IP**: 172.234.217.219
- **SSH**: `sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219`
- **Web app**: http://172.234.217.219 (nginx serving /var/www/willow/)
- **Relay**: systemd service `willow-relay` on ports 9090 (TCP) and 9091 (WebSocket)

## Steps

### 1. Build the relay server (if changed)

```bash
cargo build --release -p willow-relay
```

### 2. Build the Leptos web app

```bash
cd crates/web && trunk build --release && cd ../..
```

### 3. Upload to server

```bash
# Upload relay binary (only if rebuilt)
sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no target/release/willow-relay root@172.234.217.219:/usr/local/bin/willow-relay

# Upload web app
sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no crates/web/dist/* root@172.234.217.219:/var/www/willow/

# Fix permissions
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'chmod 644 /var/www/willow/*'
```

### 4. Restart relay (only if binary was updated)

```bash
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'systemctl restart willow-relay'
```

### 5. Verify

```bash
# Check web app
curl -s -o /dev/null -w "Web: HTTP %{http_code}\n" http://172.234.217.219/

# Check relay
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'systemctl is-active willow-relay'
```

## Quick deploy (web only)

For CSS/component changes that don't touch the relay:

```bash
cd crates/web && trunk build --release && cd ../.. && sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no crates/web/dist/* root@172.234.217.219:/var/www/willow/ && sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'chmod 644 /var/www/willow/*'
```
