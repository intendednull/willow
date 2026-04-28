# Deployment Infrastructure Design

- **Status:** Draft
- **Date:** 2026-04-28
- **Author:** Willow team
- **Branch:** `claude/research-deployment-infrastructure-V4ilv`

## Context

The Willow production stack currently runs on a single Linode VM
(`172.234.217.219`) configured by hand. Deployments happen via
`.claude/skills/deploy/SKILL.md`: build release binaries on a developer's
laptop, `scp` them to the VM, restart systemd units. The root SSH password is
committed to the repo. There is no IaC, no CI deploy path, no TLS, no
backups, no staging environment, and no rollback story. Linode's network has
also been intermittently flaky in interactive use.

This document specifies a replacement: a reproducible, declaratively-managed
deployment built on Hetzner Cloud + NixOS, provisioned and updated from
version-controlled code.

Observability (metrics, dashboards, alerts) is **out of scope** for this
spec; it is tracked in
[issue #460](https://github.com/intendednull/willow/issues/460) and will
land as a follow-up that targets the NixOS modules introduced here.

## Goals

1. **Reproducibility.** Any team member can rebuild the production host from
   scratch using only the contents of this repo plus Hetzner credentials and
   the agenix decryption key. Server state is derivable from `git`.
2. **Auditability.** Every change to production is a commit. `nixos-rebuild`
   diffs show exactly what is being modified.
3. **Atomic deploy + rollback.** Failed deploys auto-revert. No partially
   applied state.
4. **No secrets in plaintext.** SSH keys, identity material, and any future
   API tokens live encrypted in the repo.
5. **TLS by default.** All HTTP-shaped traffic served via Let's Encrypt.
6. **Survives host loss.** Identity keys and `storage.db` persist on a
   detachable volume; `restic` snapshots cover the rest.
7. **Cheap enough to ignore.** Target steady-state cost ≤ €15/mo for the
   single-host MVP.

## Non-goals

- Multi-region active/active. Single region (Hetzner Falkenstein, EU) for
  the MVP. Multi-region story sketched in §"Future work" but deferred.
- Kubernetes / containers in production. NixOS systemd units run the
  binaries directly. Docker remains a dev convenience only.
- Auto-scaling. Workload is small and bounded; capacity decisions stay
  manual.
- Application metrics + alerting (see issue #460).
- Migrating the dev stack. `just dev` and `docker-compose.yml` continue to
  work as-is.

## Topology

```
                                ┌──────────────────────────────────────┐
                                │  Hetzner Cloud — Falkenstein (FSN1)  │
                                │                                      │
   peers ──── HTTPS/443 ──────► │  caddy ─► /var/www/willow (web)      │
   peers ──── WSS/443 ────────► │  caddy ─► 127.0.0.1:9091 (relay WS)  │
   peers ──── TCP/9090 ───────► │  willow-relay (iroh TCP)             │
                                │                                      │
                                │  willow-replay  ─┐                   │
                                │  willow-storage ─┼─► relay (loopback)│
                                │                  │                   │
                                │  /var/lib/willow ─► Hetzner Volume   │
                                │  /etc/willow     ─► Hetzner Volume   │
                                └──────────────────────────────────────┘
                                          │
                                          ▼
                              Hetzner Storage Box (restic)
```

One CPX21 instance (3 dedicated AMD vCPU, 4 GB RAM, 80 GB NVMe, 20 TB
egress, ~€7.05/mo) runs everything. A 10 GB Hetzner Volume holds
`/etc/willow` (identity keys) and `/var/lib/willow` (storage SQLite). The
volume is provisioned separately from the VM so the VM can be destroyed and
recreated without losing identity or history.

## Stack at a glance

| Layer | Choice | Replaces / overrides |
|---|---|---|
| Cloud host | Hetzner Cloud (CPX21 in FSN1) | Linode VM |
| Persistent storage | Hetzner Volume (10 GB) | VM-local disk |
| Operating system | NixOS 25.05 | Hand-configured Debian |
| OS install | `nixos-anywhere` + `disko` | Manual ISO + `apt` |
| Cloud provisioning | OpenTofu + `hetznercloud/hcloud` | Hand-clicking Linode UI |
| DNS | Cloudflare (free tier) + `cloudflare/cloudflare` provider | None today |
| Configuration push | `deploy-rs` | `scp` + `systemctl restart` |
| Secrets | `agenix` | Plaintext password in skill file |
| Reverse proxy + TLS | Caddy (NixOS module) | None (HTTP only) |
| Static web hosting | Caddy on the same VM (Phase 1) | nginx serving `/var/www/willow` |
| CI | GitHub Actions | None |
| Nix binary cache | Cachix (free tier) | None |
| Backups | `restic` → Hetzner Storage Box | None |
| Edge firewall | Hetzner Cloud Firewall | None |
| In-host firewall | NixOS `networking.firewall` | None |

The remainder of this document explains each choice: what the technology
does, what role it plays in the Willow deploy, and why it was chosen over
the obvious alternatives.

## Decisions

### 1. Host: Hetzner Cloud (CPX21, Falkenstein)

**What it is.** A European IaaS provider offering virtualized x86 (AMD) and
ARM (Ampere) instances with hourly billing and per-second tear-down. CPX21
is the dedicated-vCPU AMD line: 3 vCPU, 4 GB RAM, 80 GB NVMe, 20 TB egress,
~€7.05/mo.

**Role in Willow.** Hosts the relay, both worker binaries, the static web
bundle, and Caddy. Single-host MVP; design supports adding peers later
without code changes.

**Why chosen.**

- **Network reliability.** Linode (the current host) has been observably
  flaky in interactive sessions; Hetzner's reputation for stable bandwidth
  and low jitter is the direct response to that pain point.
- **NixOS support.** Hetzner is the de-facto NixOS community standard.
  `nixos-anywhere` works against Hetzner's rescue mode in one command. The
  community has battle-tested every edge case.
- **$/perf.** A CPX21 ships 3 dedicated AMD vCPU + 20 TB egress at half the
  cost of comparable DigitalOcean / Vultr instances. Egress matters for a
  P2P relay.
- **Footprint.** Hetzner Cloud Firewall, Volumes, Floating IPs, and
  Storage Boxes are all OpenTofu-addressable, so the entire production
  surface lives in one provider's API.

**Rejected alternatives.**

- *Linode (status quo).* Network flakiness is the trigger for this work.
- *DigitalOcean.* Reliable, similar feature set, but ~2× the price for
  equivalent specs and weaker NixOS community footprint.
- *Vultr.* Cheap, supports custom ISOs, but smaller NixOS community and
  patchier support.
- *Equinix Metal.* Bare-metal global anycast story is appealing but ~5× the
  cost. Justified only above ~1k concurrent peers.
- *Fly.io.* Built-in metrics tempting, but iroh's QUIC/UDP behaviour on
  Fly's network is unproven and persistent-volume + multi-binary
  topology is awkward to express. Vendor lock-in higher.
- *AWS / GCP.* Cost and operational complexity disproportionate to a
  three-binary deployment.
- *Oracle Cloud free tier.* Useful as throwaway staging, not production:
  free-tier reclamation risk and reliability variance.

**Risks.**

- Hetzner abuse desk is strict. P2P workloads are accepted, but if a peer
  is ever flagged as a relay for abusive traffic, response time matters.
  Mitigation: keep an OpenTofu config that can re-provision on a different
  cloud in <30 minutes if needed.
- EU-only single-region until Phase 2. US peers will see higher RTT to the
  relay; acceptable for an MVP.

### 2. Persistent storage: Hetzner Volume (10 GB)

**What it is.** Network-attached block storage, provisioned as an
independent resource and attached to a server by ID. Survives server
deletion. ~€0.40/mo for 10 GB.

**Role in Willow.** Mounted at `/var/lib/willow-data`, with bind mounts to
`/etc/willow` (identity keys) and `/var/lib/willow` (storage worker SQLite
DB). The VM itself can be destroyed and recreated; identity and history
persist.

**Why chosen.**

- **Decouples lifecycle of state from compute.** Re-provisioning the VM
  (e.g. NixOS major upgrade, disaster recovery, CPU upgrade) becomes safe.
- **Same-vendor, same-DC = low latency, no egress cost.**
- **Declaratively managed via OpenTofu.** No accidental loss from
  imperative tooling.

**Rejected alternatives.**

- *VM-local disk only.* Loses everything on rebuild; couples state lifetime
  to compute lifetime.
- *S3 / object storage for SQLite.* Wrong tool — SQLite needs POSIX
  semantics. Object storage is for backups (see §10), not live state.
- *Hetzner Storage Box mounted via CIFS/NFS.* Higher latency, not designed
  for live DB writes.

### 3. Operating system: NixOS 25.05

**What it is.** A Linux distribution where the entire system configuration
— kernel, users, services, packages, network, firewall — is described as a
function from a Nix expression to a system closure. Activating a new
configuration is atomic: GRUB gets a new boot entry; failed activations
roll back via the previous generation.

**Role in Willow.** Defines the host completely. Every relevant aspect of
the production server (systemd units for relay/replay/storage, Caddy
config, mount points, firewall rules, package set, sshd config, swap)
lives in `nix/` modules in the repo.

**Why chosen.**

- **Reproducibility.** A bit-identical machine can be rebuilt from the
  flake. No "what version of nginx is on prod?" investigation.
- **Atomic upgrades.** `nixos-rebuild switch` either succeeds completely or
  rolls back. No half-applied upgrades. Fits the "no shortcuts, no
  hacky workarounds" rule from `CLAUDE.md`.
- **Declarative systemd.** The relay/replay/storage units described in
  the current `deploy` skill become `systemd.services.willow-relay = { … }`
  expressions — type-checked, linted, version-controlled.
- **Rust-friendly.** `crane` and `naersk` build Rust artefacts as Nix
  derivations, sharing a binary cache between dev and CI. Optional, not
  required for Phase 1; release binaries can be uploaded as build outputs.
- **Longevity.** NixOS configurations from 2018 still build today with
  minimal churn. The "longevity beats convenience" principle in
  `CLAUDE.md` is well-served.

**Rejected alternatives.**

- *Debian / Ubuntu + Ansible.* Mutable host, drift over time, requires
  Ansible to keep state truthful. Two-tool stack instead of one.
- *Debian + shell scripts (status quo).* No audit trail, no rollback,
  drift inevitable.
- *Fedora CoreOS / Flatcar.* Immutable, but rpm-ostree / Ignition are
  optimised for container workloads; running raw Rust binaries is a less
  natural fit and the community for non-container workloads is smaller.
- *Alpine.* Smaller, but musl + Rust + iroh has historically required
  workarounds; not worth it for ~50 MB of disk savings.

### 4. OS install: `nixos-anywhere` + `disko`

**What it is.**

- `disko` describes disk partitioning, filesystems, and mount points as a
  Nix expression. The same expression both formats disks at install and
  configures `fileSystems.*` at runtime, so the two cannot drift.
- `nixos-anywhere` is a CLI that takes any reachable Linux machine
  (including Hetzner's rescue system), `kexec`s it into the NixOS
  installer, runs `disko` to partition, then installs a NixOS
  configuration — all from one command on a developer laptop.

**Role in Willow.** First-time provisioning of a fresh Hetzner VM:

```sh
nix run github:nix-community/nixos-anywhere -- \
  --flake .#willow-prod \
  --target-host root@<hetzner-rescue-ip>
```

Re-runnable. Idempotent within a generation.

**Why chosen.**

- **No ISO mounting, no PXE.** Hetzner doesn't offer a clean "boot from
  custom ISO" flow for cloud instances. `nixos-anywhere` sidesteps this
  by using rescue mode, which Hetzner provides natively.
- **Disk layout in version control.** The same `disko` expression is the
  source of truth for partition table, LUKS (if used), filesystem types,
  and mount options. No imperative `parted` runs.
- **One command from zero to running NixOS.** Replaces the multi-step
  Hetzner-rescue dance documented in countless blog posts.

**Rejected alternatives.**

- *`nixos-infect`.* In-place conversion of an existing distro. Works, but
  the resulting layout is whatever the cloud image gave you — partition
  tables aren't part of the Nix config.
- *Manual NixOS install via rescue.* Slow, error-prone, not reproducible.
- *Hetzner cloud-init image.* Mutable, drifts.

### 5. Cloud provisioning: OpenTofu + `hetznercloud/hcloud`

**What it is.**

- **OpenTofu** is the Linux Foundation fork of Terraform 1.5, kept under
  MPL after HashiCorp's BSL relicense. CLI-compatible: `tofu init`,
  `tofu plan`, `tofu apply`. The `terraform-provider-hcloud` provider
  works unchanged.
- The provider exposes Hetzner Cloud resources (servers, volumes,
  firewalls, floating IPs, networks, SSH keys, placement groups) as
  declarative Terraform/OpenTofu blocks.

**Role in Willow.** Defines every Hetzner resource — VM, attached volume,
firewall, reserved IP — in `terraform/` (or `tofu/`) HCL. `tofu apply` is
the only way changes are made to Hetzner's API. State is stored remotely
in a Hetzner Storage Box (encrypted) so it isn't trapped on a developer
laptop.

**Why chosen.**

- **MPL licence.** No commercial-use ambiguity from HashiCorp's BSL move.
  Aligns with willow's open-source stance.
- **Drop-in compatibility.** The wider ecosystem (`hcloud`, `cloudflare`,
  `null`, `random`) all work; same `.tf` files an existing Terraform user
  would write.
- **Boring + maintained.** OpenTofu is now backed by a large coalition
  including IBM, Oracle, Spacelift; well past the "will this fork
  survive?" stage.

**Rejected alternatives.**

- *Terraform proper (HashiCorp).* BSL licence; fine for internal use today
  but historically the kind of "speculative future bet" `CLAUDE.md` warns
  against locking in.
- *Pulumi.* Excellent ergonomics (Rust SDK exists), but the team's
  surface area is mostly Rust + Nix; introducing a TypeScript/Go IaC
  language is gratuitous tooling.
- *`hcloud` CLI scripts.* Imperative; no plan/diff; no state file; no
  drift detection.
- *Ad-hoc Nix expressions calling Hetzner's REST API.* Possible
  (`terranix` exists), but reinvents what OpenTofu already does well.

### 6. DNS: Cloudflare (free tier) + `cloudflare/cloudflare` provider

**What it is.** Authoritative DNS hosted by Cloudflare, managed
declaratively via the same OpenTofu config that provisions the Hetzner VM.
A-records for `willow.<domain>`, `relay.<domain>`, optional CNAMEs, all
TTLs and proxy settings live in HCL.

**Role in Willow.**

- `willow.<domain>` → VM IPv4/IPv6 (web UI).
- `relay.<domain>` → VM IPv4/IPv6 (WSS endpoint via Caddy on :443).
- `_acme-challenge.*` → managed by Caddy automatically; no manual records.

**Why chosen.**

- **Free, fast, mature provider.** Cloudflare's OpenTofu provider is
  among the best-supported.
- **Decoupled from the host.** Switching cloud providers later changes a
  handful of `A` record values, not registrar config.
- **Optional proxy.** For the web UI specifically, Cloudflare's proxy can
  be turned on with a flag flip, giving CDN + DDoS protection for free.
  The relay's WSS / TCP traffic stays direct (proxy would terminate
  TLS in a way that breaks the WS upgrade path, and TCP/9090 is
  iroh's binary protocol — not proxyable at all).

**Rejected alternatives.**

- *Hetzner DNS.* Functional, but slower API and weaker tooling. Pinning
  DNS to the same vendor as compute also hurts the "swap providers in 30
  min" risk mitigation.
- *Route 53.* Paid, requires AWS account creep for one feature.
- *Self-hosted bind / NSD.* Adds operational surface for zero benefit.

### 7. Configuration push: `deploy-rs`

**What it is.** A NixOS deploy tool by Serokell. Given a flake describing
target hosts and their NixOS configurations, `deploy-rs` builds the closure
locally (or pulls from a binary cache), copies it to the target via SSH,
and runs `nixos-rebuild switch` over a control channel. Two features
distinguish it: parallel deploys to multiple hosts and **magic rollback** —
after activating a new generation, the tool checks that the SSH
control connection still works; if not (e.g. firewall misconfig locked
itself out), the host auto-reverts to the previous generation.

**Role in Willow.**

```
$ deploy .#willow-prod        # build, ship, switch, verify
```

CI runs the same command on push to `main` after the test gate passes.
Magic-rollback ensures a botched firewall rule or sshd change cannot
brick the box.

**Why chosen.**

- **Auto-rollback is the differentiator.** The current deploy skill has no
  rollback; a bad systemd unit or firewall change requires manual recovery
  via Hetzner console. `deploy-rs` removes that risk class.
- **Flakes-first.** Reads `nixosConfigurations.<name>` from the same flake
  the rest of the repo uses. No separate "deploy DSL" to learn.
- **Lightweight.** Single Rust binary, no daemon, no controller. Runs from
  CI or laptop interchangeably.
- **Parallel multi-host friendly.** Phase 2 multi-region will deploy 2–3
  hosts concurrently with one command, no script glue.

**Rejected alternatives.**

- *Colmena.* Excellent tool, mature, simpler config than `deploy-rs`. Lacks
  magic-rollback. For a solo-operator project where mistakes are likely,
  the rollback safety net wins.
- *NixOps.* Effectively unmaintained; the canonical successor (NixOps 2)
  never reached production maturity.
- *`morph`.* Similar to colmena but smaller community.
- *`nixos-rebuild --target-host`.* Built into NixOS; works fine for
  one-off deploys but no rollback story and no multi-host orchestration.
- *Bash + `nix copy` + `ssh nixos-rebuild switch`.* Reinvents `deploy-rs`
  with worse error handling.

**Risks.**

- `deploy-rs` is single-maintainer-ish. If it stalls, migration to
  `colmena` is a one-day port (same flake structure). Tracked as an
  acceptable risk.

### 8. Secrets: `agenix`

**What it is.** A NixOS module + small CLI that encrypts secret files with
[age](https://age-encryption.org/) keys derived from each host's SSH
ed25519 key. Encrypted blobs (`.age` files) live in the repo. At boot, the
NixOS activation script decrypts them into `/run/agenix/<name>` (tmpfs,
mode 0400, owned by the consuming service's user). No daemon, no runtime
HTTP call, no external KMS.

**Role in Willow.**

- Encrypts the `restic` repository password.
- Encrypts the Hetzner API token used by OpenTofu (when run from CI).
- Encrypts the Cloudflare API token.
- Encrypts the Cachix auth token (CI side).
- (Optional) Encrypts the long-lived peer identity keys used by the relay
  and workers, so they can be reproduced on a fresh volume from git
  history. Decision deferred — see "Open questions".

`secrets.nix` lists which `.age` files are decryptable by which host
public keys; rekeying a secret to a new host is one CLI call.

**Why chosen.**

- **Secrets in git, audit trail intact.** Every change to a secret is a
  reviewable commit; rotation is a `git log`-able event.
- **Zero runtime infrastructure.** Decryption happens at boot using the
  host's existing SSH key. No Vault to operate, no AWS account, no
  network call.
- **Per-host scoping.** A leaked dev laptop key doesn't unlock prod
  secrets unless that key was explicitly added to `secrets.nix`.
- **Fits NixOS atomic activation.** Secrets land before services start;
  if decryption fails, activation fails — no half-up service.

**Rejected alternatives.**

- *`sops-nix`.* Comparable feature set, supports KMS backends (AWS, GCP,
  age, PGP). More flexibility, more configuration. For a single-cloud
  deploy with no existing KMS investment, `agenix` is simpler. Migration
  path exists if KMS becomes desirable.
- *HashiCorp Vault.* Heavyweight: requires running Vault, unsealing,
  managing tokens. Disproportionate for ~5 secrets.
- *1Password / Bitwarden + CLI fetching at deploy.* Adds runtime vendor
  dependency and a non-Nix code path.
- *Plaintext files outside the repo (current state).* Already rejected
  upstream — the password leak in `.claude/skills/deploy/SKILL.md` is the
  reason this work exists.

**Migration note.** The committed root password
(`WillowP2P2026deploy!`) is **rotated and revoked** as part of Phase 0
of the migration (see §"Implementation phases"). All future SSH access
is key-based; the deploy skill is rewritten to consume `deploy-rs`.

### 9. Reverse proxy + TLS: Caddy

**What it is.** A single-binary HTTP/2/3 server written in Go with
automatic Let's Encrypt + ZeroSSL certificate provisioning. The NixOS
module (`services.caddy`) accepts a `Caddyfile` (or structured Nix
expressions producing one) and handles renewal, OCSP stapling, and
HTTP→HTTPS redirects.

**Role in Willow.** Single ingress for all HTTP-shaped traffic on the VM:

```
willow.<domain>      :443 → /var/www/willow      (static web, HTTP/3)
relay.<domain>       :443 → 127.0.0.1:9091       (relay WebSocket via WSS)
relay.<domain>       :80  → ACME challenge + redirect to :443
```

iroh's TCP/9090 endpoint is **not** behind Caddy — it's iroh's own binary
QUIC-over-TCP framing, not HTTP. Caddy doesn't see it. The Hetzner Cloud
Firewall opens 9090/tcp directly to the relay process.

**Why chosen.**

- **Automatic TLS, no certbot timer wrangling.** Caddy renews via ACME,
  retries on failure, has been doing this for years. NixOS module exposes
  `enableACME` boolean style configuration, removing manual cron.
- **Single static binary.** Tiny config surface; the entire reverse-proxy
  layer is ~30 lines of Caddyfile.
- **HTTP/3 + QUIC support out of the box.** Forward-compatible with the
  rest of Willow's transport stack.
- **WebSocket upgrade is a default.** No special config to proxy WSS to
  the relay's WS port; unlike nginx where it requires explicit
  `Upgrade`/`Connection` header passthrough.

**Rejected alternatives.**

- *nginx + certbot.* Works, but the cert-renewal lifecycle is a separate
  systemd timer + hook contract. More moving parts; more places for the
  current "no TLS" problem to recur.
- *Traefik.* Designed for container/Kubernetes-shaped service discovery;
  overkill on a static-host topology.
- *HAProxy.* No native ACME; would still need certbot.
- *Letting the relay terminate TLS itself.* The relay is purpose-built for
  iroh transport; bolting in HTTPS termination + ACME renewal would mix
  concerns and inflate its surface area.

### 10. Static web hosting: Caddy on the same VM (Phase 1)

**What it is.** The Leptos web app builds to a `dist/` directory of
static HTML/CSS/WASM/JS via `trunk build --release`. In Phase 1, those
files are deployed to `/var/www/willow` on the VM and served by Caddy
from the same `:443` ingress.

**Role in Willow.** Browser users hit `https://willow.<domain>`, get the
WASM bundle from Caddy, then the bundle establishes WSS to
`https://relay.<domain>` (also Caddy → relay).

**Why this layering.**

- **Single deploy unit.** The web bundle is part of the same NixOS
  closure as everything else; one `deploy-rs` invocation atomically
  updates frontend + backend, eliminating the
  "frontend updated, backend didn't" failure mode the current
  scp-based flow can produce.
- **Same TLS cert path.** No separate CDN cert / origin cert juggling.
- **Trivial to migrate later.** The web bundle is plain static files;
  swapping to Cloudflare Pages or similar is a DNS change plus a CI
  artefact upload step.

**Phase 2 follow-up: Cloudflare Pages.** When sustained traffic warrants
a CDN, move `/var/www/willow` content to Cloudflare Pages:

- Web traffic is then served from Cloudflare's edge (200+ POPs);
  VM bandwidth is freed for relay/worker traffic.
- Web deploys decouple from VM deploys (different release cadence).
- Pages free tier is generous (unlimited bandwidth, 500 builds/mo).

This isn't done in Phase 1 because: (a) the VM has 20 TB/mo egress to
spare; (b) keeping web on-VM in Phase 1 means the spec ships with one
fewer external dependency to coordinate.

**Rejected alternatives for Phase 1.**

- *Cloudflare Pages from day one.* Not wrong, but introduces a second
  deploy target and CI workflow before the rest of the stack is stable.
  Defer.
- *S3 + CloudFront.* AWS account + IAM, more moving parts, no advantage
  over Caddy-on-VM at current scale.
- *Netlify / Vercel.* Comparable to Pages; chosen against because
  Cloudflare is already the DNS provider, so Pages reuses that account.

### 11. CI: GitHub Actions

**What it is.** GitHub-hosted CI/CD runners triggered by repository
events (push, pull_request, tag, manual). Workflows live in
`.github/workflows/*.yml`.

**Role in Willow.** Two workflows in the new model:

1. `check.yml` — runs on every PR. Executes `just check-all` (fmt,
   clippy, all crate tests, WASM build, browser tests via wasm-pack,
   Playwright). The existing PR gate.

2. `deploy.yml` — runs on push to `main` after `check.yml` passes.
   Steps:
   - Install Nix via `cachix/install-nix-action`.
   - Authenticate to Cachix (push side).
   - `nix build .#willow-prod-system` — builds the entire NixOS closure.
     Cachix populates from the build.
   - `nix run .#deploy -- .#willow-prod` — `deploy-rs` ships the
     pre-built closure to the production host via SSH using a CI-only
     deploy key stored in GitHub Secrets.

**Why chosen.**

- **Already where the code lives.** Repo is on GitHub; no extra
  account/billing.
- **Free for public repos** and generous for private (~2000 min/mo).
- **First-class Nix support** via `cachix/install-nix-action` (well
  maintained by the Cachix maintainers).
- **Secret management** via GitHub repo Secrets is sufficient for the
  small set of tokens needed (Hetzner, Cloudflare, Cachix push, deploy
  SSH key). Sensitive infrastructure secrets stay in `agenix`; only
  CI-side credentials live in GitHub.

**Rejected alternatives.**

- *Self-hosted runners (e.g. on the production VM).* Recursive deploy
  topology; if the runner host is unhealthy you can't deploy a fix.
- *SourceHut builds.* Excellent for Nix workflows but moves CI off
  GitHub for marginal benefit.
- *Garnix.* Promising Nix-native CI but newer; revisit when the cache
  + UI story matures.
- *Buildkite / CircleCI.* Paid, no compelling advantage.

**Risks.**

- Trust boundary: a GitHub compromise means deploy access. Mitigated by
  scoping the deploy SSH key to only run `nix-store --import` +
  `switch-to-configuration`, not interactive shells. Tracked as residual
  risk.

### 12. Nix binary cache: Cachix (free tier)

**What it is.** A hosted binary cache for Nix derivations. Build once
(in CI), push the resulting `/nix/store` paths to Cachix, then any other
machine (CI, dev laptop, prod) can pull pre-built artefacts instead of
recompiling. Free tier: 5 GB.

**Role in Willow.**

- CI's `nix build` step pushes its outputs to a `willow` Cachix cache.
- The production host's `nix.settings.substituters` includes the same
  cache, so `deploy-rs` ships only the closure-diff over SSH (most
  store paths are already cached).
- Developer laptops opt in via `cachix use willow`, getting CI-built
  rust/leptos/iroh artefacts for free.

**Why chosen.**

- **Order-of-magnitude faster CI and deploys.** A cold rust + iroh
  build is 10+ minutes; a cache hit is seconds. CI flake-ier feedback
  loops are corrosive over time.
- **Lowest-friction option.** No infra to operate; sign up, add a
  GitHub Action step, done.
- **Compatible with self-host upgrade path.** If the 5 GB free tier is
  ever exhausted, switching to a self-hosted [`attic`](https://github.com/zhaofengli/attic)
  cache is a one-line change to `substituters` and `trusted-public-keys`.

**Rejected alternatives.**

- *Self-hosted `attic`.* Higher operational burden for Phase 1; revisit
  if Cachix free tier proves insufficient.
- *No cache.* Acceptable for tiny projects; not for one with a Rust
  workspace + WASM + iroh + Leptos. CI minutes burn fast.
- *S3 + `nix-serve-ng`.* DIY equivalent; reinvents Cachix.

**Risk.** Cachix is a single vendor. If it disappears, builds still
work — just slower until `attic` is stood up. No correctness risk.

### 13. Backups: `restic` → Hetzner Storage Box

**What it is.**

- **`restic`** is a Rust-era-style backup tool (actually Go) with content-
  addressed, deduplicated, encrypted snapshots. Each snapshot is
  encrypted with a single repository password; only changed chunks are
  uploaded.
- **Hetzner Storage Box** is a per-account network storage product
  (SFTP/SSH/Borg/SMB/WebDAV) starting at €3.49/mo for 1 TB. Resides in
  Hetzner's network, so backup traffic from the VM is free egress.

**Role in Willow.** A nightly systemd timer (`services.restic.backups.willow`)
snapshots:

- `/etc/willow` — peer identity keys (relay, replay, storage).
- `/var/lib/willow` — storage worker SQLite DB.
- `/var/lib/caddy` — TLS cert state (renewable; included for fast
  recovery rather than correctness).

Retention policy via `restic forget`:
`--keep-daily 7 --keep-weekly 4 --keep-monthly 12`.

The repository password and Storage Box SSH key are managed by
`agenix`. `restic` integrity checks (`restic check --read-data-subset 5%`)
run weekly to detect bit rot.

**Why chosen.**

- **Encryption + dedup as defaults.** Identity keys are sensitive; an
  unencrypted backup defeats the point of `agenix`.
- **Same-vendor target = no egress cost, low latency.**
- **NixOS module exists** (`services.restic.backups.<name>`) — fully
  declarative including timers, paths, retention, and pre/post hooks.
- **Restore tested as part of migration.** §"Implementation phases"
  Phase 4 includes a documented restore drill.

**Rejected alternatives.**

- *`borgbackup`.* Comparable feature set; Hetzner Storage Box even
  supports a native Borg endpoint. Restic chosen because: (a) its
  cloud-target story is broader, easing future provider portability;
  (b) the `restic` CLI is more ergonomic for ad-hoc inspection.
- *`rsnapshot` / `rsync` snapshots.* No dedup, no encryption.
- *Backblaze B2 / S3-compatible target.* Cheaper per GB, but adds a
  separate vendor and small egress costs. Storage Box wins at this scale.
- *Volume snapshots only (Hetzner UI).* Hetzner offers volume snapshots,
  but they're full-volume images managed via the cloud API, not granular
  file-level restores. Useful as a coarse safety net (kept on, free for
  one snapshot per volume), insufficient as the primary backup.

### 14. Firewalls: Hetzner Cloud Firewall + NixOS in-host

**What they are.**

- **Hetzner Cloud Firewall** is an L3/L4 stateful firewall enforced at
  Hetzner's network edge, before traffic reaches the VM. Configured
  declaratively via OpenTofu.
- **NixOS `networking.firewall`** is `nftables`-based, runs on the host
  itself, and is part of the NixOS configuration.

**Role in Willow.** Defense in depth.

Edge (Hetzner) — explicit allow list:

| Port | Protocol | Source | Purpose |
|---|---|---|---|
| 22 | TCP | deploy CI IPs + (optional) admin allowlist | SSH |
| 80 | TCP | 0.0.0.0/0 | ACME HTTP-01 challenge + redirect |
| 443 | TCP + UDP | 0.0.0.0/0 | HTTPS + HTTP/3 |
| 9090 | TCP | 0.0.0.0/0 | iroh relay TCP |
| 9091 | TCP | 127.0.0.1/32 (loopback) | relay WS — only Caddy reaches it |

In-host (NixOS) — same allow list as a fallback, plus `fail2ban` style
rate limits on SSH brute force via `services.fail2ban`.

**Why two layers.**

- Edge firewall drops abusive traffic before it consumes VM bandwidth.
- In-host firewall is the source of truth in `nix/` modules (so the
  rules are auditable in the same diff as the rest of the config) and
  protects against misconfiguration of the cloud firewall (e.g. someone
  accidentally widening it via the Hetzner UI).
- The two are kept in sync because both are generated from the same
  Nix-side definition: a small helper exports the rule list to both an
  OpenTofu locals file and the NixOS firewall config.

**Why chosen.**

- **Minimal cost.** Hetzner Cloud Firewall is free.
- **No new tooling.** Both layers are already produced from the IaC the
  rest of the spec uses.

**Rejected alternatives.**

- *Edge-only firewall.* Single point of misconfig.
- *In-host-only firewall.* Wastes bandwidth on dropped traffic.
- *Cloudflare-as-firewall.* Cloudflare's WAF is HTTP-aware; can't filter
  iroh's TCP/9090. Not the right tool here.

## Repository layout

New top-level directories introduced by this work:

```
infra/
├── flake.nix                  # NixOS configurations + deploy-rs
├── nix/
│   ├── modules/
│   │   ├── willow-relay.nix      # systemd unit + user + paths
│   │   ├── willow-replay.nix
│   │   ├── willow-storage.nix
│   │   ├── caddy.nix             # reverse proxy + TLS
│   │   ├── firewall.nix          # in-host nftables (source of truth)
│   │   ├── backup.nix            # restic + Storage Box
│   │   └── observability.nix     # placeholder, owned by issue #460
│   ├── hosts/
│   │   └── willow-prod.nix       # CPX21 in FSN1, imports modules
│   └── disko/
│       └── cpx21.nix             # disk + volume layout
├── tofu/
│   ├── main.tf                # hcloud server, volume, firewall, FIP
│   ├── dns.tf                 # cloudflare records
│   ├── variables.tf
│   └── outputs.tf
└── secrets/
    ├── secrets.nix            # who-can-decrypt-what
    ├── restic-password.age
    ├── hcloud-token.age       # CI uses; not deployed to host
    └── cloudflare-token.age   # CI uses; not deployed to host

.github/workflows/
├── check.yml                  # PR gate (existing `just check-all`)
└── deploy.yml                 # main → build → cachix → deploy-rs

scripts/
└── (existing dev.sh stays untouched)
```

The `.claude/skills/deploy/SKILL.md` is rewritten as a thin wrapper:
"Run `nix run .#deploy -- .#willow-prod` after `tofu apply`. Secrets are
managed via agenix; see `infra/secrets/`."

## Implementation phases

The migration is sequenced so each phase produces a working system and
leaves a clean rollback point.

### Phase 0 — Lockdown (day 1, ≤ 1 hour)

- Rotate the root password on the current Linode VM; remove it from
  `.claude/skills/deploy/SKILL.md` (commit removal).
- Replace password auth with an SSH key pinned to a temporary admin user.
- Stop committing future deploy material.

This is independent of the rest and reduces immediate risk while the new
stack is built.

### Phase 1 — Greenfield Hetzner box (1–2 days)

- Stand up `infra/tofu` + `infra/nix` skeleton.
- Provision Hetzner CPX21 + Volume + Firewall + Cloudflare records under
  a staging hostname (e.g. `willow-staging.<domain>`).
- Run `nixos-anywhere` once.
- Deploy relay/replay/storage as fresh peers with new identities.
- Verify Caddy + TLS on the staging hostname, run a peer end-to-end via
  the agent harness against the staging relay.

Outcome: production parity at the staging hostname; production
(Linode) still serving real users.

### Phase 2 — Cutover (≤ 1 hour, scheduled window)

- Migrate the **relay's** Ed25519 identity from Linode to the new Hetzner
  volume, so the relay's peer ID is preserved across the cutover and
  existing peers' connection state remains valid. One-time `scp` of
  `/etc/willow/relay.key` followed by immediate revocation of the
  source machine's SSH access. Workers (`replay`, `storage`) get
  **fresh** identities on Hetzner; SyncProvider permission is re-granted
  to them via a single state event after they connect. This is simpler
  than migrating worker keys and aligns with the worker model
  (workers are operationally fungible; relays are addressing
  endpoints).
- Re-point DNS in Cloudflare to the new IP. Pre-stage with low TTL
  (60s) the day before so propagation is effectively instant.
- Decommission Linode VM after a 7-day soak.

### Phase 3 — Backups (≤ 2 hours)

- Provision Hetzner Storage Box.
- Add `services.restic.backups.willow` to the NixOS module set.
- Run a manual snapshot, then a documented restore drill into a fresh
  CPX11 to prove the recipe works end-to-end. This is the only
  "verify before claiming done" step the spec hard-requires (per
  `superpowers:verification-before-completion`).

### Phase 4 — CI deploy path (1 day)

- Add `.github/workflows/deploy.yml`.
- Move from "developer laptop runs `deploy-rs`" to "merge to `main`
  triggers deploy".
- Keep the laptop path working as a break-glass procedure.

### Phase 5 — Observability (deferred, issue #460)

- Track separately. Targets the same NixOS modules and reuses `agenix`
  for monitoring secrets.

## Cost summary (steady state)

| Item | EUR/mo |
|---|---|
| Hetzner CPX21 (compute) | 7.05 |
| Hetzner Volume (10 GB) | 0.40 |
| Hetzner Floating IP (IPv4) | 0.50 |
| Hetzner Storage Box (1 TB) | 3.49 |
| Cloudflare DNS | 0.00 |
| Cachix (free tier) | 0.00 |
| GitHub Actions (public repo or under free tier minutes) | 0.00 |
| **Total** | **~11.44** |

Comfortably under the €15/mo goal. Adds ~€7/mo for a second Hetzner
host when Phase 2 multi-region is undertaken.

## Resolved decisions

These were open during initial research and are now settled. Recorded
here so the trail of "why this and not that" is auditable.

1. **Identity-key continuity at cutover.** **Resolved:** migrate the
   relay's Ed25519 key from Linode to Hetzner so the relay peer ID is
   preserved and existing peers reconnect transparently. Workers
   (`replay`, `storage`) get fresh identities — SyncProvider is
   re-granted via a single state event after they're online. Rationale:
   relays are addressable endpoints (peer ID is user-visible state);
   workers are operationally fungible. See Phase 2 cutover for the
   migration mechanics.
2. **Domain name.** **Resolved:** Cloudflare manages DNS for whichever
   domain Willow is using. Registrar stays put (no transfer in scope).
   The OpenTofu config takes the zone name as a variable so the spec is
   not coupled to a specific domain.
3. **Identity keys in `agenix`.** **Resolved:** **no.** Identity keys
   stay on the Hetzner Volume only, with `restic` snapshots as the
   recovery path. Encrypting and committing them would put high-value
   long-lived secrets in git history; the operational risk outweighs
   the convenience of "rebuild from `git checkout`". Volume restore
   from `restic` is the documented recovery procedure.
4. **CI deploy key scope.** **Resolved:** start with a
   `forced-command` SSH key in `authorized_keys`, restricted to the
   `nix-copy-closure` + `switch-to-configuration` operations
   `deploy-rs` performs. Full interactive SSH is the break-glass
   procedure (separate per-developer key, not the CI key). Documented
   in the runbook section of the implementation plan.

## Future work

These are tracked separately and explicitly out of scope here.

- **Multi-region (EU + US relays).** Add a second Hetzner host in
  Ashburn, share state via Willow's own gossip, route DNS by GeoIP via
  Cloudflare. Identity keys per-host stay distinct; both granted
  SyncProvider.
- **Cloudflare Pages for the web bundle** (Phase 1 follow-up).
- **Self-hosted `attic` cache** if Cachix free tier is exhausted.
- **Observability stack** — see issue #460.
- **Disaster-recovery game day.** Schedule a periodic exercise where the
  prod VM is intentionally destroyed and rebuilt from `infra/` + restic;
  measure recovery time. Target: <30 minutes.
- **Hardware key for `agenix`.** Eventually move the per-developer age
  identities onto YubiKeys (`age-plugin-yubikey`).

## Appendix A: Why not Kubernetes / containers in production?

Earlier discussion considered Kamal (Docker over SSH) and various
Kubernetes flavours (k3s, k0s, managed). They were rejected because:

- Willow is **three Rust binaries + a static bundle**. Containerising
  them adds an OCI layer with no operational payoff at this scale.
- NixOS systemd units already give all the lifecycle features Kamal /
  Kubernetes provide (restart policy, dependency ordering, atomic
  upgrades, isolation via DynamicUser).
- Docker on the production host is *kept available* for ad-hoc
  troubleshooting parity with `docker-compose.yml`, but no production
  service runs through it.
- If Phase-2 multi-region grows toward many heterogeneous hosts and
  rolling upgrades become a concern, this decision is revisitable. The
  NixOS modules wrap binaries; swapping to OCI images is mechanical.

## Appendix B: Tradeoff summary

The decisions above optimise for **longevity, auditability, and minimal
operational surface**, in that order — matching the project's stated
values in `CLAUDE.md` ("quality + longevity beat speed + convenience").

The largest accepted cost is the **NixOS learning curve**: the team
must become fluent in flakes, modules, and `nixos-rebuild` semantics.
This is paid once and pays back forever in eliminated drift, atomic
deploys, and reproducibility.

The largest deferred cost is **observability**: critical for operating
the deployed system, intentionally split out so the deploy spec can
land first. Issue #460 is the contract.
