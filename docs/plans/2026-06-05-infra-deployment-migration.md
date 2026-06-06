# Plan — Migrate Willow deployment onto the `infra` NixOS flake

**Date:** 2026-06-05 (live: 2026-06-06)
**Status:** active — **deployed + live.** `willow.intendednull.com` (web) + `relay.willow.intendednull.com`
serve valid LE certs; the app mounts and the WASM client connects through the relay (browser-verified).
Two extra web-build fixes were needed for static serving (see R1/R5). Remaining: open PRs, swap infra's
`willow` input `git+file`→`git+ssh`, decommission the old box (`172.237.156.18`).
**Owner:** Noah (operator) + agent
**Related:** `../infra/docs/specs/2026-06-04-deploy-infra-design.md` (target platform),
`../infra/ONBOARDING.md` (the contract), `../infra/docs/specs/2026-06-05-multi-process-capability-design.md`
(the `runtime="multi"` capability willow uses).

Supersedes the ad-hoc `.claude/skills/deploy` skill (sshpass → `172.234.217.219`, manual
systemd + peer-id `sed`). That whole mechanism is **retired** by this plan.

---

## Goal

Deploy the full Willow stack (web UI + relay + replay + storage) as **one declarative
`runtime="multi"` app on the shared `infra` Hetzner box**, replacing the hand-rolled
sshpass/systemd deploy. End state: `just deploy web` (from `infra`) builds the closure on the
dev box, pushes it, and the `/healthz` gate verifies every unit; the public site is live at
`https://willow.intendednull.com` once DNS is pointed at the box.

## Decisions (locked)

| Decision | Choice | Why |
|---|---|---|
| App shape | **single `runtime="multi"` app** named `willow` | replay/storage are port-less P2P workers — no single-process runtime fits them; multi is the only home, and one repo = one app is coherent |
| Web domain | `willow.intendednull.com` (registry-fronted) | matches the fleet pattern; primary user surface = registry `domain` |
| Relay domain | `relay.willow.intendednull.com` (module-added 2nd Caddy vhost) | browser is on an HTTPS page → mixed-content rules forbid `ws://` → relay **must** be `wss://` → TLS-terminated by Caddy. Raw public TCP (old 9090/9091 model) is dead. |
| Relay bind | `127.0.0.1:3340`, Caddy reverse-proxies with WS streaming (`flush_interval -1`) | fits the single-edge-terminator model; no firewall changes |
| Web serving | loopback static file server unit (SPA fallback), registry-fronted | mirrors the infra `static` template; keeps web health-gated + primary domain conventional |
| Relay/worker health | relay `/bootstrap-id` (already 200); replay/storage **port-less** units (gated by `after=` only, like `canary-multi-seed`) | no app code needed for healthz; workers have no HTTP by design |
| Identity keys + DB | `StateDirectory=willow` → `/var/lib/willow/{relay,replay,storage}.key`, `storage.db`; generated on first boot | persists across deploys; matches current auto-generate behavior; **not** sops (node-local, regenerable) |
| Web build | crane `buildTrunkPackage`, `wasm-bindgen-cli` pinned to **0.2.118** (Cargo.lock) | hermetic; the only fiddly bit is the wbg version pin |
| Old artifacts | **full cleanup**: delete `docker/`, `docker-compose.yml`, the sshpass deploy skill, docker-* justfile targets | per operator decision; infra is the sole deploy path |
| DNS | operator adds A records (web + relay → box public IP) | no registrar automation on the box; same as the healinggrief cutover |

## The three identities (infra onboarding trap)

| role | value |
|---|---|
| flake-input == registry-key == app/module name (triple-match) | `willow` |
| cargo packages (`-p`) | `willow-relay`, `willow-replay`, `willow-storage`, `willow-web` |
| `[[bin]]` names | `willow-relay`, `willow-replay`, `willow-storage` (web = trunk dist, not a bin) |

---

## Workstreams

### A. Willow repo — Nix packaging + module (this worktree)

- **A1.** Add `flake.nix` (multi app): `inputs` = nixpkgs `nixos-26.05`, crane, rust-overlay
  (all `follows nixpkgs`). `packages.${system}`:
  - `relay`, `replay`, `storage` = `craneLib.buildPackage` per `-p` crate (`doCheck = false`,
    asset-aware `src` not needed for the servers — no embedded assets).
  - `web` = `craneLib.buildTrunkPackage` of `crates/web` (trunk source incl. `*.css`, `init.js`,
    `manifest.json`, icons, `sw.js`, chime). Pin `wasm-bindgen-cli` to 0.2.118.
  - `default` = `symlinkJoin` of the four (contract wants `packages.default`; multi doesn't need
    `packages.oci` — moonlitmorphs omits it).
  - `nixosModules.default = import ./nix/module.nix self`.
- **A2.** `nix/module.nix` (closes over `self`) — declares `options.services.willow` (`enable`,
  `webDomain`, `relayDomain`, `webPort` default 8093, `relayPort` default 3340) and:
  - fixed system user/group `willow` (shared `StateDirectory` for keys + db).
  - `systemd.services.willow-web` — static server on `127.0.0.1:${webPort}` serving `self…web`
    with SPA fallback; ships/serves `/healthz`.
  - `systemd.services.willow-relay` — `${self…relay}/bin/willow-relay --relay-port ${relayPort}
    --identity /var/lib/willow/relay.key`, `127.0.0.1` bind.
  - `systemd.services.willow-replay` — `--identity-path /var/lib/willow/replay.key --relay-url
    http://127.0.0.1:${relayPort}`; `After=willow-relay`.
  - `systemd.services.willow-storage` — `--identity-path …/storage.key --db-path …/storage.db
    --relay-url http://127.0.0.1:${relayPort}`; `After=willow-relay`.
  - hardening per template (`ProtectSystem=strict`, `NoNewPrivileges`, `CapabilityBoundingSet=[""]`,
    `StateDirectory=willow`).
  - `services.caddy.virtualHosts.${relayDomain}` → `reverse_proxy 127.0.0.1:${relayPort}` with
    WS tuning. (mkApp fronts the **web** domain via the registry port; the relay vhost is the
    module's own addition.)
- **A3.** Fix `crates/web/src/app.rs`: `DEFAULT_RELAY_URL` → `wss://relay.willow.intendednull.com`
  (current `https://willow.intendednull.com:9443` is stale). Update the unit test if any pins the
  old value.
- **A4.** Full cleanup: delete `docker/`, `docker-compose.yml`, the `docker-*` justfile targets,
  and **`.github/workflows/deploy.yml`** (the old sshpass CI auto-deploy). Grep-sweep
  `172.234.217.219` (leave the two historical plan/spec docs — rewriting past records falsifies
  them; *supersede* the worker-nodes spec's deployment section with a banner instead).
- **A5.** **Delete** the willow `deploy` skill + any justfile deploy shim entirely — deployment is
  owned solely by `infra` (a willow-side skill would only `cd ../infra`, a false path assumption in
  CI/fresh clones). Willow keeps only the **flake** (its app-packaging contract that infra consumes).
  Document the deploy command as a one-line pointer in `README.md` + `CLAUDE.md`.
- **A6.** `just check` green; commit on the worktree branch.

### B. Infra repo — onboard `willow` (`/mnt/storage/projects/infra`)

- **B1.** Add `willow` flake input. Inner loop: `git+file:///mnt/storage/projects/willow?ref=<branch>`;
  before finalizing: `git+ssh://git@github.com/intendednull/willow` (push willow first). `follows nixpkgs`.
- **B2.** Registry entry (`hosts/registry.nix`):
  ```nix
  willow = {
    runtime = "multi"; domain = "willow.intendednull.com"; port = 8093; healthz = "/health";
    relayDomain = "relay.willow.intendednull.com";
    units = [
      { unit = "willow-web";     port = 8093; healthz = "/health"; }       # public (mkApp vhost)
      { unit = "willow-relay";   port = 3340; healthz = "/bootstrap-id"; } # module adds relay vhost
      { unit = "willow-replay"; }                                          # port-less P2P worker
      { unit = "willow-storage"; }                                         # port-less P2P worker
    ];
  };
  ```
- **B3.** Wire `services.willow` in `hosts/web/default.nix` (enable + pass `webDomain`/`relayDomain`
  from the registry, like moonlitmorphs).
- **B4.** `nix flake update willow`; `nix flake check`; preflight eval of each unit's `ExecStart`.
- **B5.** Onboarding report `docs/reports/2026-06-05-onboarding-willow-trial.md` (friction log,
  mirroring the healinggrief report) + registry/ONBOARDING edits if new friction surfaces.

### C. Deploy + go live

- **C1.** `just deploy web` over the tailnet → activation + `/healthz` gate (web `:8093/healthz`,
  relay `:3340/bootstrap-id`; replay/storage gate on `after=`).
- **C2.** Read the box **public IP** off the box (over tailnet); hand the operator the two A records
  (`willow` + `relay.willow` → IP). Use LE **staging** in Caddy while iterating.
- **C3.** After DNS resolves: flip Caddy to prod ACME, `just deploy web` again, then smoke test:
  - `https://willow.intendednull.com` serves the app (200).
  - `https://relay.willow.intendednull.com/bootstrap-id` → 200 text.
  - **Two browsers connect through the relay and sync a message** — validates the relay's
    `advertised_addr=127.0.0.1` hardcode actually works behind Caddy (the one real runtime risk).
- **C4.** Update `docs/README.md` indexes (willow + infra). Mark this plan `landed`.

## Risks / open items — resolved

- **R1 (build).** ✅ `wasm-bindgen-cli` pinned to 0.2.118 via `buildWasmBindgenCli` (nixpkgs ships
  0.2.121). Also needed: `CC/AR_wasm32` → unwrapped LLVM so `ring` compiles its C for wasm; run
  trunk from `crates/web` (crane runs from the workspace root → "could not find the root package").
- **R2 (runtime).** ✅ **Cleared.** The deployed WASM client connects through the Caddy-fronted relay
  (active iroh-gossip in a real browser, no errors) — the hardcoded `advertised_addr=127.0.0.1`
  works behind Caddy. No relay code change needed.
- **R3 (relay vhost).** ✅ The module-added `relay.willow.intendednull.com` vhost composes with
  mkApp's web vhost (different domains, no collision); both serve valid LE certs.
- **R4 (DNS).** ✅ Operator added A records (web + relay → `5.78.195.207`). Caddy needed a manual
  `systemctl reload caddy` to clear the ACME backoff from the deploy-before-DNS sequence.
- **R5 (static serving, web).** ✅ Two trunk-static blockers fixed in the flake's web build, **without
  touching willow's source CSP/index.html**: (a) `--no-sri=true` — trunk SRI fails under the
  static-web-server zstd compression; (b) `nix/externalize-bootstrap.py` — willow's CSP
  `script-src 'self'` (no `'unsafe-inline'`) blocks trunk's inline bootstrap, so it's lifted to an
  external `/trunk-bootstrap.js` (allowed by `'self'`). **Latent willow bug** (the app can't be
  served as static files under its own CSP); proper long-term fix belongs in willow's web build.
