# Hardened NixOS module for willow — a `runtime = "multi"` app (../infra ONBOARDING §2b).
#
# Declares its OWN four systemd units (mkApp does NOT synthesize services.<name> for a
# multi app — it only imports this module and fronts the registry `port` via the edge
# Caddy). Closes over the flake's `self` so it can reach self.packages.<system>.* (a plain
# module can't see pkgs.willow — Discourse #18492).
#   nixosModules.default = import ./nix/module.nix self;
#
# Units:
#   willow-web     — loopback static server (SPA fallback), edge-Caddy-fronted at webDomain
#                    (the registry public port). Bind 127.0.0.1:webPort.
#   willow-relay   — iroh relay + gossip bootstrap on 0.0.0.0:relayPort. This module adds a
#                    SECOND edge-Caddy vhost (relayDomain → wss://) since browsers on an HTTPS
#                    page can't use plaintext ws://. Health = GET /bootstrap-id (200).
#   willow-replay  — port-less P2P worker, connects to the relay over loopback. after=relay.
#   willow-storage — port-less P2P worker + SQLite archive. after=relay.
#
# Persistent state (keys + storage.db) lives in StateDirectory=willow → /var/lib/willow,
# shared by a fixed system user (NOT DynamicUser — a stable uid keeps the shared dir + keys
# coherent across units and restarts).
self:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.willow;
  sys = pkgs.stdenv.hostPlatform.system;
  webPkg = self.packages.${sys}.web;
  relayBin = lib.getExe' self.packages.${sys}.relay "willow-relay";
  replayBin = lib.getExe' self.packages.${sys}.replay "willow-replay";
  storageBin = lib.getExe' self.packages.${sys}.storage "willow-storage";

  stateDir = "/var/lib/willow";

  # Common hardening + identity for every willow unit (spec §7 common rules).
  common = {
    User = "willow";
    Group = "willow";
    StateDirectory = "willow";
    Restart = "always";
    RestartSec = 5;
    ProtectSystem = "strict";
    ProtectHome = true;
    PrivateTmp = true;
    NoNewPrivileges = true;
    CapabilityBoundingSet = [ "" ];
  };
in
{
  options.services.willow = {
    enable = lib.mkEnableOption "willow P2P stack (web + relay + replay + storage)";
    webDomain = lib.mkOption {
      type = lib.types.str;
      description = "Public web UI domain (registry domain; edge Caddy fronts webPort here).";
    };
    relayDomain = lib.mkOption {
      type = lib.types.str;
      description = "Public relay domain — this module adds an edge Caddy vhost (wss://) for it.";
    };
    webPort = lib.mkOption {
      type = lib.types.port;
      default = 8093;
      description = "Loopback port the static web server binds (must equal the registry port).";
    };
    relayPort = lib.mkOption {
      type = lib.types.port;
      default = 3340;
      description = "Port the relay binds (loopback-reachable; edge Caddy fronts relayDomain here).";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.willow = { isSystemUser = true; group = "willow"; home = stateDir; };
    users.groups.willow = { };

    systemd.services.willow-web = {
      description = "willow web UI (static loopback file server)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      environment.RUST_LOG = lib.mkDefault "info";
      serviceConfig = common // {
        # --health adds a dedicated GET /health → 200 (no log spam) for the deploy gate.
        # --page-fallback serves index.html for unknown paths → SPA client-side routing.
        ExecStart = lib.escapeShellArgs [
          (lib.getExe pkgs.static-web-server)
          "--host" "127.0.0.1"
          "--port" (toString cfg.webPort)
          "--root" "${webPkg}"
          "--page-fallback" "${webPkg}/index.html"
          "--health"
          "--log-level" "info"
        ];
        MemoryMax = "128M";
      };
    };

    systemd.services.willow-relay = {
      description = "willow iroh relay + gossip bootstrap node";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      environment.RUST_LOG = lib.mkDefault "info";
      serviceConfig = common // {
        # --identity load-or-generates + persists the bootstrap key (mode 0600) in StateDir.
        ExecStart = lib.escapeShellArgs [
          relayBin
          "--relay-port" (toString cfg.relayPort)
          "--identity" "${stateDir}/relay.key"
        ];
        MemoryMax = "512M";
      };
    };

    systemd.services.willow-replay = {
      description = "willow replay worker (bounded in-memory state sync)";
      wantedBy = [ "multi-user.target" ];
      after = [ "willow-relay.service" "network.target" ];
      wants = [ "willow-relay.service" ];
      environment.RUST_LOG = lib.mkDefault "info";
      serviceConfig = common // {
        ExecStart = lib.escapeShellArgs [
          replayBin
          "--identity-path" "${stateDir}/replay.key"
          "--relay-url" "http://127.0.0.1:${toString cfg.relayPort}"
          "--max-events-per-author" "1000"
          "--sync-interval" "30"
        ];
        MemoryMax = "1G";
      };
    };

    systemd.services.willow-storage = {
      description = "willow storage worker (SQLite archival history)";
      wantedBy = [ "multi-user.target" ];
      after = [ "willow-relay.service" "network.target" ];
      wants = [ "willow-relay.service" ];
      environment.RUST_LOG = lib.mkDefault "info";
      serviceConfig = common // {
        ExecStart = lib.escapeShellArgs [
          storageBin
          "--identity-path" "${stateDir}/storage.key"
          "--db-path" "${stateDir}/storage.db"
          "--relay-url" "http://127.0.0.1:${toString cfg.relayPort}"
          "--sync-interval" "60"
        ];
        MemoryMax = "1G";
      };
    };

    # Second edge vhost for the relay (mkApp fronts only the registry web port). Caddy
    # terminates TLS + transparently upgrades the WebSocket; flush_interval -1 disables
    # response buffering for the long-lived relay stream.
    services.caddy.virtualHosts.${cfg.relayDomain}.extraConfig = ''
      reverse_proxy 127.0.0.1:${toString cfg.relayPort} {
        flush_interval -1
      }
      header Strict-Transport-Security "max-age=31536000; includeSubDomains"
    '';
  };
}
