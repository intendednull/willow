#!/bin/sh
set -e
mkdir -p /etc/willow

exec willow-relay --relay-port 3340 --identity /etc/willow/relay.key
