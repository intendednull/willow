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
7. **Cheap enough to ignore.** Target steady-state cost ≤ €20/mo for the
   single-host MVP. (The €15 number floated during research predated the
   2026-04-01 Hetzner price adjustment; revised after price-check.)

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

One CAX21 ARM instance (4 vCPU Ampere, 8 GB RAM, 80 GB NVMe, 20 TB
egress, ~€7.99/mo post-2026-04-01 pricing) runs everything. A 10 GB
LUKS-encrypted Hetzner Volume holds `/etc/willow` (identity keys) and
`/var/lib/willow` (storage SQLite). The volume is provisioned
separately from the VM so the VM can be destroyed and recreated
without losing identity or history.

## Stack at a glance

| Layer | Choice | Replaces / overrides |
|---|---|---|
| Cloud host | Hetzner Cloud (CAX21 ARM in FSN1) | Linode VM |
| Persistent storage | Hetzner Volume (10 GB, LUKS-encrypted) | VM-local disk |
| Operating system | NixOS 25.11 ("Xantusia") | Hand-configured Debian |
| OS install | `nixos-anywhere` + `disko` | Manual ISO + `apt` |
| Cloud provisioning | OpenTofu + `hetznercloud/hcloud` | Hand-clicking Linode UI |
| DNS | Cloudflare (free tier) + `cloudflare/cloudflare` provider | None today |
| Configuration push | `deploy-rs` | `scp` + `systemctl restart` |
| Secrets | `agenix` | Plaintext password in skill file |
| Reverse proxy + TLS | Caddy (NixOS module) | None (HTTP only) |
| Static web hosting | Caddy on the VM (committed) | manual `scp` to Linode VM, no TLS |
| CI | GitHub Actions | None |
| Nix binary cache | Cachix (free tier) | None |
| Backups | `restic` → Hetzner Storage Box | None |
| Edge firewall | Hetzner Cloud Firewall | None |
| In-host firewall | NixOS `networking.firewall` | None |

The remainder of this document explains each choice: what the technology
does, what role it plays in the Willow deploy, and why it was chosen over
the obvious alternatives.

## Decisions

### 1. Host: Hetzner Cloud (CAX21 ARM, Falkenstein)

**What it is.** A European IaaS provider offering virtualized x86 (AMD)
and ARM (Ampere) instances with hourly billing and per-second tear-down.
CAX21 is the ARM Ampere shared-vCPU line: 4 vCPU, 8 GB RAM, 80 GB NVMe,
20 TB egress, **€7.99/mo** (post-2026-04-01 pricing).

A CAX21 (ARM) is preferred over CPX21 (AMD) for two reasons:
1. **Better $/perf after the April 2026 price adjustment.** CPX21 went
   from €8.99 → €11.99/mo. CAX21 stayed at €7.99/mo with one more vCPU
   and 4 GB more RAM than CPX21.
2. **Willow already targets aarch64.** The Rust workspace builds clean
   for `aarch64-unknown-linux-gnu`; iroh + leptos have no x86-only
   dependencies. WASM build is target-independent.

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
- **$/perf.** A CAX21 ships 4 ARM vCPU + 8 GB + 20 TB egress at less than
  half the cost of comparable DigitalOcean / Vultr instances. Egress
  matters for a P2P relay.
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

- Hetzner abuse desk is strict and famously fast on complaints. The risk
  vectors that actually trigger lockdowns are *port scans*, *DMCA
  notices*, and *outbound spam* — relay-style P2P itself is not banned
  by the AUP. Mitigation: keep an OpenTofu config that can re-provision
  on a different cloud in <30 minutes if needed; egress firewall rules
  in NixOS prevent the relay from being reused for outbound abuse if
  ever compromised.
- EU-only single-region in Phase 1. US peers will see higher RTT to the
  relay; acceptable for an MVP. Hetzner now has Singapore (SIN) and
  Hillsboro / Ashburn (US) options for Phase-2 multi-region.
- Hetzner Cloud account compromise = full takeover (rescue mode reads
  the volume). See §"Security baseline" for required 2FA + token
  scoping controls.

### 2. Persistent storage: Hetzner Volume (10 GB, LUKS-encrypted)

**What it is.** Network-attached block storage, provisioned as an
independent resource and attached to a server by ID. Survives server
deletion. ~€0.57/mo for 10 GB at €0.0572/GB/mo (post-2026-04-01).

**Role in Willow.** Mounted at `/var/lib/willow-data`, with bind mounts to
`/etc/willow` (identity keys) and `/var/lib/willow` (storage worker SQLite
DB). The VM itself can be destroyed and recreated; identity and history
persist.

**LUKS encryption.** The volume is formatted with LUKS2 (declared in
`disko`). Hetzner Volumes are not encrypted at rest by default; an
internal Hetzner storage incident or a misrouted detach/attach event
would otherwise expose the relay's Ed25519 identity key in cleartext.
The LUKS key is stored in `/run/agenix/willow-volume.key` (decrypted at
boot from agenix), not in TPM — the CAX21 doesn't expose one to guests
— so unlock requires the host SSH key, consistent with the rest of the
secret model. Disaster-recovery note: the LUKS recovery key is
maintained offline alongside the restic password (see §13).

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

### 3. Operating system: NixOS 25.11 ("Xantusia")

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

**nixpkgs pinning.** `flake.lock` pins `nixpkgs` to a specific commit
hash, **not** the `nixos-25.11` channel name. Channel-name following
introduces silent drift on each `nix flake update`; commit-hash pinning
makes upgrades explicit reviewable diffs. The version-bump procedure
(staging → smoke test → prod) is documented in §"Host configuration
baseline".

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
- `_acme-challenge.*` → managed by Caddy via DNS-01 using a Cloudflare
  API token (scoped `Zone.DNS:Edit` on the willow zone only). No
  manual records, no port-80 dependency, works regardless of whether
  the A record is proxied.
- `CAA` records pinning Let's Encrypt as the only permitted issuer
  (with `accounturi` constraint to a known ACME account). Prevents an
  attacker who briefly compromises the Cloudflare zone from getting a
  cert from a different CA.

**Why chosen.**

- **Free, fast, mature provider.** Cloudflare's OpenTofu provider is
  among the best-supported.
- **Decoupled from the host.** Switching cloud providers later changes a
  handful of `A` record values, not registrar config.
- **Optional proxy.** For the web UI specifically, Cloudflare's proxy
  can be turned on with a flag flip, giving CDN + DDoS protection for
  free. The relay's traffic stays **direct** (unproxied) for two
  reasons: (a) Cloudflare imposes a 100s idle timeout on free/pro
  plans which would forcibly close long-lived gossip WSS
  connections, and (b) iroh's TCP/9090 is a binary protocol that
  Cloudflare's HTTP-aware proxy cannot route at all.

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
- *`nh` (nix-community/nh).* Modern Rust rewrite of `nixos-rebuild` /
  `home-manager switch`, in nixpkgs stable as of v4. Has `--target-host`
  deploy support and good diff visualisation. Lacks magic-rollback —
  the same reason colmena is rejected. Re-evaluate once a comparable
  rollback story lands upstream.
- *`clan-core` (clan.lol).* Umbrella Nix project bundling
  `nixos-anywhere` + `disko` + `sops-nix` + secret management +
  deploy. Coherent and well-maintained, but bundles `sops-nix` (vs
  agenix) and adopts opinions about machine ownership we don't need.
  Worth reconsidering if our own toolchain composition becomes
  burdensome to maintain.

**Risks.**

- `deploy-rs` is single-maintainer-ish, but commits remain steady
  through 2026-Q1. If it stalls, migration to `colmena` is a one-day
  port (same flake structure). Tracked as an acceptable risk.
- See §11 for the more substantial CI threat model: `deploy-rs` ships
  arbitrary Nix closures to prod, which run activation scripts as root.
  Constraints on the SSH command are a no-op against a malicious
  closure. Closure signing + manual approval are required mitigations,
  not optional ones.

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
  reviewable commit. **Caveat:** the *previous ciphertext* of any rotated
  secret remains in git history forever — agenix rotation must therefore
  always include rotation at the source-of-truth (e.g. regenerate the
  Hetzner API token in the Hetzner panel) so the history copy is dead
  material. Recorded as a procedure invariant in the runbook.
- **Zero runtime infrastructure.** Decryption happens at boot using the
  host's existing SSH key. No Vault to operate, no AWS account, no
  network call.
- **Per-host scoping.** A leaked dev laptop key doesn't unlock prod
  secrets unless that key was explicitly added to `secrets.nix`.
- **Fits NixOS atomic activation.** Secrets land before services start;
  if decryption fails, activation fails — no half-up service.

**Bootstrap chicken-and-egg.** A fresh `nixos-anywhere` install
generates a new host SSH key during installation; that key's age public
form must be in `secrets.nix` for any agenix-encrypted blob to decrypt.
The bootstrap sequence is therefore:

1. Provision the VM via OpenTofu.
2. Run `nixos-anywhere --no-substitute-on-destination` with a *minimal*
   configuration that has no agenix secrets. This boots a working host
   and produces `/etc/ssh/ssh_host_ed25519_key.pub`.
3. SSH in, capture the host key fingerprint and the derived age public
   key (`ssh-to-age < /etc/ssh/ssh_host_ed25519_key.pub`).
4. Add the public key to `secrets.nix`, run `agenix --rekey` for every
   `.age` blob, commit.
5. Run `deploy-rs` to switch the host to its full configuration with
   secrets present.

This is documented in the migration runbook (Appendix C).

**Secret rotation propagation.** When a secret is rotated, its
ciphertext changes but consumers don't restart automatically. Each
service consuming an agenix secret must declare:

```nix
restartTriggers = [ config.age.secrets.<name>.file ];
```

so that `deploy-rs` registers the changed file path as a unit
input and triggers `systemctl restart` on the consumer. Tested in
Phase 1 by rotating the restic password and confirming the backup unit
restarts.

**Threat model — agenix host-key compromise is forever-decrypting.**
Anyone who reads `/etc/ssh/ssh_host_ed25519_key` once (Hetzner rescue
mode, panel-initiated snapshot, lateral movement, restic backup of the
SSH key) can decrypt **every** agenix secret ever committed under that
host's age identity, past and future. agenix has no online revocation
primitive; rotation requires regenerating the host key + rekeying every
blob, and every leaked-era secret must be treated as permanently
disclosed at the source-of-truth.

Mitigations:

- **Exclude `/etc/ssh/ssh_host_ed25519_key` from the restic backup
  set.** Confirmed in §13. The host key must not be recoverable via
  backups; if the box is destroyed, the new box gets a new host key,
  and the rekey-everything procedure runs.
- **Hardware-token 2FA + scoped API tokens** on the Hetzner panel
  (see §"Security baseline") so rescue-mode read of the host key
  requires multi-factor compromise.
- **Smaller blast radius on rotation:** secrets that genuinely need
  online revocation (e.g. third-party API tokens that an attacker could
  exfiltrate and use immediately) live in agenix; secrets where
  rotation is acceptable annual maintenance (TLS certs, intra-cluster
  bearer tokens) also fit. For very high-value secrets where this
  threat model is uncomfortable, `sops-nix` with hardware-token age
  identities is the upgrade path (see "Rejected alternatives" below).

**Rejected alternatives.**

- *`sops-nix`.* Comparable feature set, supports KMS backends (AWS, GCP,
  age, PGP) and per-secret access control. The *real* tradeoff vs.
  agenix isn't "more configuration" but **revocability** — sops-nix can
  use age identities held on YubiKeys or in cloud KMS, which support
  revocation; agenix's host-key model does not. For Phase 1 we accept
  the agenix tradeoff because the secret set is small and rotation is
  cheap; if the secret set grows or includes secrets that *cannot* be
  rotated at source, migration to sops-nix is the documented next step.
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

- **Automatic TLS, no certbot timer wrangling.** Caddy renews via ACME
  using **DNS-01** with the Cloudflare token from agenix; this is
  resilient to a future decision to put the web UI behind Cloudflare
  proxy and removes the port-80 dependency for renewal.
- **Single static binary.** Tiny config surface; the entire reverse-proxy
  layer is ~30 lines of Caddyfile.
- **HTTP/3 + QUIC support out of the box.** Forward-compatible with the
  rest of Willow's transport stack.
- **WebSocket upgrade is a default.** No special config to proxy WSS to
  the relay's WS port; unlike nginx where it requires explicit
  `Upgrade`/`Connection` header passthrough.

**Caddy footguns we explicitly handle.**

- **WASM MIME type.** Caddy's stock build does not auto-serve
  `.wasm` files as `application/wasm`. The Leptos bundle's streaming
  WASM compile fails silently if served as `text/html`. The Caddyfile
  declares:
  ```caddyfile
  @wasm path *.wasm
  header @wasm Content-Type application/wasm
  ```
- **Brotli precompression.** Caddy stock ships gzip + zstd encoders
  but **not Brotli**. `trunk build --release` produces `.br`
  precompressed bundles by default. Two paths:
  1. (Phase 1) Use a NixOS overlay that builds Caddy with the
     `caddy-brotli` plugin via `xcaddy`. Documented; ~15 lines.
  2. (Alternative) Configure `trunk` to skip Brotli, ship gzip + zstd
     only. ~3% larger over-the-wire than Brotli; acceptable.
  The Phase-1 build uses path (1) for parity with the existing
  `trunk` defaults.
- **Admin API.** Caddy's admin endpoint defaults to `localhost:2019`.
  We disable it explicitly with `admin off` in the Caddyfile; we don't
  use admin-API config reload (NixOS-managed configs only) and even
  loopback exposure is unnecessary attack surface.

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

### 10. Static web hosting: Caddy on the VM (committed, not transitional)

**What it is.** The Leptos web app builds to a `dist/` directory of
static HTML/CSS/WASM/JS via `trunk build --release`. Files are deployed
to `/var/www/willow` on the VM and served by Caddy from the same `:443`
ingress.

**Role in Willow.** Browser users hit `https://willow.<domain>`, get the
WASM bundle from Caddy, then the bundle establishes WSS to
`https://relay.<domain>` (also Caddy → relay).

**Why this is the long-term answer, not a stepping stone.**

The original draft framed this as "Phase 1 Caddy-on-VM, Phase 2
Cloudflare Pages." That was a shortcut — the kind `CLAUDE.md`
explicitly warns against — because it deferred a real cost
(coordinating two deploy targets, two cert paths, version skew between
the WASM bundle and the relay protocol) for an unproven future benefit
("CDN" without measured need).

The committed answer is: **Caddy on the VM, with Cloudflare proxy on
the `willow.<domain>` A record** when CDN is desired. This gets:

- **Single deploy unit.** The web bundle ships in the same NixOS
  closure as the relay/workers. `deploy-rs` activates them
  atomically; "frontend updated, backend didn't" cannot happen.
- **Same TLS cert path.** Caddy's DNS-01 + Cloudflare token issues
  one cert; the proxy reuses it as origin cert. No CDN-cert
  juggling.
- **CDN if and when needed.** Toggling Cloudflare's orange-cloud
  proxy is a one-line OpenTofu change. No new vendor account, no new
  build target, no new failure mode. Worker / relay traffic continues
  to bypass the proxy.
- **20 TB/mo Hetzner egress** is room enough that even a viral
  growth event doesn't force the issue.

If at some point the bundle outgrows what one CAX21 can comfortably
serve, the migration to a dedicated edge product is then a real,
measured decision — not a speculative split today.

**Rejected alternatives.**

- *Cloudflare Pages.* Worth revisiting only if the bundle ever needs
  global edge serving with sub-50ms TTFB everywhere. Today, the
  Cloudflare proxy on top of Caddy gives 90% of that for free.
- *S3 + CloudFront.* AWS account + IAM, more moving parts, no advantage.
- *Netlify / Vercel.* Comparable to Pages; same logic.

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

**Threat model — CI deploy = root on prod.** Honest framing matters
here. The naive read of "CI runs `nix-copy-closure` + `switch-to-
configuration` via a forced-command SSH key" suggests CI has restricted
access. It does not. A NixOS closure can contain arbitrary code, and
`switch-to-configuration` runs activation scripts as root. Anyone who
can push a closure to the host and trigger activation has root on the
host. The forced-command restriction is a defence against accidental
shell access, not against a malicious deploy.

This means: **a GitHub Actions runner compromise = full prod
compromise**, equivalent to a leaked root SSH key. The same applies to
a Cachix push-key compromise (attacker substitutes a malicious closure)
and to a `cachix/install-nix-action` supply-chain compromise (attacker
runs malicious code with deploy-key access).

Required mitigations:

- **Manual approval gate.** `deploy.yml` uses a GitHub
  [environment](https://docs.github.com/en/actions/deployment/targeting-different-environments/using-environments-for-deployment)
  named `production` with required reviewers. Deploys do not auto-run
  on push; a human approves the run after CI passes.
- **Pin actions by SHA, not tag.** All third-party actions (notably
  `cachix/install-nix-action`) referenced by full commit SHA, with
  `dependabot` watching for upstream version bumps so review happens
  before adoption.
- **Closure signing.** The deploy artefact is signed with a Nix
  signing key whose **public** half is in production's
  `nix.settings.trusted-public-keys` and whose **private** half lives
  in GitHub Secrets at `WILLOW_NIX_SIGNING_KEY`, scoped to a single
  job (`build-and-sign`) within `deploy.yml`. Production's
  `trusted-public-keys` lists *only* this key — Cachix's key is
  permitted as a *substituter* for unprivileged store paths but not
  as a trust root for what gets activated.

  **What this defends:** a Cachix supply-chain compromise (poisoned
  cache) is then non-fatal because prod won't activate paths Cachix
  signed but our signing key didn't. A `cachix/install-nix-action`
  compromise that runs in the *cache-fetch* job (which uses the
  Cachix key but not the signing key) is also non-fatal.

  **What this does not defend:** a full GitHub Actions runner
  compromise of the signing job still yields prod root, because that
  job legitimately holds the signing key. Closure signing is a
  blast-radius reduction — Cachix is no longer "if compromised, prod
  is rooted" — not a defence against runner compromise itself. The
  spec's threat-model claim is bounded accordingly.
- **Workflow `permissions:` minimised.** Each workflow declares
  `permissions:` explicitly (default `read` everywhere; `write`
  only where strictly required).
- **Dedicated deploy SSH key**, separate from any per-developer
  break-glass key, rotatable independently.

These are not optional hardening; they are required for the threat
model in this spec to hold.

**Risks (residual).**

- Compromise of an approver's GitHub account still gives prod root.
  Mitigated by mandating hardware-token 2FA on every approver's
  GitHub account (see §"Security baseline").
- Self-hosted runners are *not* used; their compromise model is
  worse (network-adjacent to prod).

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

**Trust boundary.** Cachix is a *substituter*, not a trust root.
Production's `nix.settings.trusted-public-keys` pins only the closure
signing key (held in agenix; see §11). Cachix's signing key is
permitted to populate `/nix/store` paths, but the activated
configuration is verified against our key, not theirs. A Cachix
compromise therefore degrades performance (poisoned-cache detection
forces rebuild) but does not yield prod root.

`narinfo-cache-negative-ttl = 0` is set so a missing-then-present
upstream change is detected promptly.

**Self-host fallback.** If Cachix is degraded or its terms change, the
self-host alternatives are:

- [`celler`](https://discourse.nixos.org/t/celler-an-attic-fork/77265)
  — a 2026-Q1 community fork of `attic`, which itself stalled around
  mid-2025. Celler is the current actively-maintained route to a
  self-hosted Nix binary cache. **Note:** the original draft cited
  `attic` as the fallback; that recommendation is stale.
- A nix-serve-ng + signed-narinfo setup, hand-rolled.

Neither is "drop in tomorrow" — both require a few hours of
provisioning. The risk note has therefore been demoted from
"one-line change" to "documented project of a day or so".

**Rejected alternatives.**

- *No cache.* Acceptable for tiny projects; not for one with a Rust
  workspace + WASM + iroh + Leptos. CI minutes burn fast.
- *S3 + `nix-serve-ng`.* DIY equivalent; viable as fallback (above)
  but more work than Cachix for Phase 1.

**Risk.** Cachix is a single vendor. If it disappears, builds still
work — just slower until `celler` is stood up. No correctness risk
because closure signing pins activation independently.

### 13. Backups: `restic` → Hetzner Storage Box

**What it is.**

- **`restic`** is a content-addressed, deduplicated, encrypted backup
  tool. Each snapshot is encrypted with a single repository password;
  only changed chunks are uploaded.
- **Hetzner Storage Box (BX11)** is a per-account network storage
  product (SFTP/SSH/Borg/SMB/WebDAV) at **€3.20/mo for 1 TB**. Resides
  in Hetzner's network, so backup traffic from the VM is free egress.

**Role in Willow.** A nightly systemd timer (`services.restic.backups.willow`)
snapshots:

- `/etc/willow` — peer identity keys (relay, replay, storage).
- `/var/lib/willow` — storage worker SQLite DB.
- `/var/lib/caddy` — TLS cert state (renewable; included for fast
  recovery rather than correctness).

The LUKS unlock material for the Hetzner Volume is **not** in the
backup set — the source of truth is the agenix-encrypted blob in git
plus the offline LUKS recovery key (see DR triplet below). Backing it
up onto the Volume itself would be circular.

**Identity keys at restore vs. cutover.** The relay's identity is
migrated *once* at Phase-2 cutover (see Appendix C) and thereafter
restored byte-identical from `restic` if the Volume is lost — peer ID
is preserved across DR. Worker (`replay`/`storage`) identities are
also restored byte-identical from `restic`; the "fresh worker
identities" decision applies only to the *initial* Hetzner build (it
saves an avoidable migration), not to subsequent disaster recovery.
SyncProvider re-grants therefore only happen at the Phase-2 cutover,
not on every DR exercise.

**Explicitly excluded** from the backup set:

- `/etc/ssh/ssh_host_*` — host SSH keys. Backing them up would
  reverse the agenix threat-model assumption that compromise of *one*
  host's SSH key only decrypts secrets for *that host's lifetime*. A
  rebuilt host gets a fresh key; the rekey-everything procedure runs
  on first deploy.
- `/var/log/journal` — chatty, recoverable from external shipping
  later.

Retention policy via `restic forget`:
`--keep-daily 7 --keep-weekly 4 --keep-monthly 12`.

**Append-only Storage Box.** The Storage Box SSH key issued to the
production host is wrapped with `restrict,command="rclone serve
restic --append-only ..."` (or equivalent rest-server config) so the
production host can write new snapshots and read existing ones, but
**cannot delete past snapshots**. A Storage Box compromise via the
production host therefore can't destroy backup history. Pruning runs
from a separate maintenance flow (developer laptop with full key,
invoked manually or via a separately-credentialed CI job).

`restic` integrity checks (`restic check --read-data-subset 5%`) run
weekly to detect bit rot.

**Disaster-recovery: full-loss scenario.** If the VM and the volume
are both lost, the chain to recover is:

```
restic password (in agenix)
  └─ decrypted by host SSH key
       └─ which lived on the destroyed volume
```

That's circular. The full bootstrap-from-zero chain also requires
decrypting the OpenTofu API tokens (`hcloud-token.age`,
`cloudflare-token.age`) before the new VM exists, which depends on
an *operator* age identity (not a host one). Four secrets are
therefore kept **offline, outside the host**, in a developer-managed
password vault (1Password, hardware token, sealed envelope — operator's
choice, but *not* in this git repo):

1. The restic repository password.
2. The Storage Box SSH key (private half) — append-only credential is
   sufficient for restore; pruning credential is held separately.
3. The LUKS recovery key for the Hetzner Volume.
4. **At least one operator age private key** listed as a recipient
   in `secrets.nix`, so a fresh laptop can decrypt the Hetzner +
   Cloudflare API tokens to drive `tofu apply`. Recommended form:
   `age-plugin-yubikey` identity on a hardware token. Accepted
   alternative: a sealed-envelope copy of the age private key.

The recovery procedure (Phase 1 drill) verifies these four are
sufficient to bring up a fresh CAX21 with the original storage state,
*starting from a laptop with no pre-existing access*. The "DR runbook"
appendix lists the exact steps.

**Backup-failure notification.** A silent backup failure is worse than
no backup. Each `restic` unit declares `OnFailure=` pointing to a
minimal `services.msmtp` + `email-notify@.service` unit that emails an
ops address from the LUKS-protected agenix-decrypted SMTP credential.
This is the only place the spec ships an email path before the full
observability stack (#460) lands. If the email isn't desired, any
webhook (Discord, ntfy, Mattermost) substitutes.

**Why chosen.**

- **Encryption + dedup as defaults.** Identity keys are sensitive; an
  unencrypted backup defeats the point of `agenix`.
- **Same-vendor target = no egress cost, low latency.**
- **NixOS module exists** (`services.restic.backups.<name>`) — fully
  declarative including timers, paths, retention, and pre/post hooks.
- **Restore tested as part of migration.** §"Implementation phases"
  Phase 1 includes a documented restore drill.

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

Edge (Hetzner) — explicit allow list, applied to both IPv4 and IPv6.
Hetzner Cloud Firewall is L3/L4 *stateful permit-only*; it does not
do per-source connection rate limiting. Rate limits in the table below
are enforced by **in-host** `nftables` rules that share the same Nix
definition.

| Port | Protocol | Source | Hetzner edge | In-host nftables |
|---|---|---|---|---|
| 22 | TCP | deploy CI IPs + admin allowlist | permit | permit + sshd `MaxAuthTries=3` |
| 80 | TCP | 0.0.0.0/0, ::/0 | permit | permit (HTTP→HTTPS redirect; ACME uses DNS-01) |
| 443 | TCP + UDP | 0.0.0.0/0, ::/0 | permit | permit |
| 9090 | TCP | 0.0.0.0/0, ::/0 | permit | per-source limit 60/min, burst 10; global `ct count` ≤ 5000 |

Port 9091 (relay WS) is **not** listed at the edge: it binds to
`127.0.0.1:9091` only and is reachable solely via Caddy's loopback
proxy. Listing it with `127.0.0.1` in an *edge* firewall would be
nonsense — loopback traffic never traverses Hetzner's network. The
relay binary's listen address is set explicitly to `127.0.0.1:9091`
in its NixOS module, so the host firewall is not load-bearing for
this confidentiality property.

**Rate limiting + DDoS posture on 9090.** Cloudflare cannot proxy
iroh's binary TCP/9090, so the only mitigations are at the host's own
NIC. The in-host `nftables` rules apply a per-source connection rate
limit and a global `ct count` cap (see table). This catches noisy
single-IP misbehaviour, not a coordinated volumetric flood — a
sufficiently large L4 attack saturates the Hetzner uplink before any
host rule sees the packet. That residual risk is acknowledged in
"Future work" with a defined trigger threshold and fallback runbook
(Floating IP swap to a temporary scrubbing host).

In-host (NixOS) — same allow list as a fallback, generated from a
single Nix definition shared with the OpenTofu locals so the two
layers cannot drift. SSH brute-force handling uses
`services.openssh.settings.MaxAuthTries=3` plus per-source rate
limits in `nftables` (chosen over `services.fail2ban` because
fail2ban's nftables backend integration is fragile on NixOS — see
upstream issue tracker).

**IPv6.** Hetzner provides a `/64` per VM. The same allow list above
applies to v6; explicit `::/0` rules. Outgoing IPv6 is permitted
without filtering; egress filtering is in scope for Phase-2
hardening.

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

## Host configuration baseline

The decisions above name *which* tools to use; this section names the
default settings every host inherits via `nix/modules/baseline.nix`.
These are the kind of settings whose absence quietly causes outages:

- **Time sync.** `services.timesyncd.enable = true` with the default
  `pool.ntp.org` servers. Willow's HLC ordering tolerates clock skew
  but degrades quality if it grows unbounded.
- **Swap.** 2 GiB `zramSwap.enable = true` with `zstd` compressor.
  Avoids OOM on transient memory spikes; CAX21's 8 GB RAM has plenty
  of headroom but a misbehaving worker shouldn't OOMkill the relay.
- **Nix garbage collection.** `nix.gc.automatic = true` with
  `options = "--delete-older-than 14d"`, weekly. Without this, the
  80 GB NVMe fills within months as `deploy-rs` ships closures.
- **Nix store optimisation.** `nix.optimise.automatic = true` weekly.
- **journald limits.** `services.journald.extraConfig =
  "SystemMaxUse=512M\nMaxRetentionSec=14d\nForwardToSyslog=no"`.
  Bounds disk usage and disables remote forwarding by default.
- **sshd hardening.** `services.openssh.settings = { PermitRootLogin
  = "no"; PasswordAuthentication = false; KbdInteractiveAuthentication
  = false; MaxAuthTries = 3; AllowUsers = [ "deploy" "ops" ]; }`.
  Keys-only, non-root, named users.
- **Mount ordering.** Each willow systemd unit has
  `RequiresMountsFor=/etc/willow /var/lib/willow` so a slow volume
  attach can't produce empty-state startup.
- **Filesystem layout.** `/etc/willow` and `/var/lib/willow` are bind
  mounts off the LUKS-decrypted Hetzner Volume; the bind mounts are
  declared in the NixOS module, not `/etc/fstab`, so they're version-
  controlled.

**NixOS version-bump procedure.** `nixpkgs` is pinned by hash in
`flake.lock`. Bumps follow:

1. Local: `nix flake update nixpkgs`, run `just check-all`, smoke
   build the closure.
2. PR: review the diff (`nvd diff` between old and new closures
   highlights notable package version changes).
3. Staging: deploy-rs to the staging hostname; run the agent harness
   end-to-end smoke test against it.
4. Soak: leave staging on the new closure for at least 24 hours.
5. Prod: deploy-rs to prod; magic-rollback armed.

Magic-rollback covers SSH-reachability failures but does **not**
detect "service started but is broken in a subtle way" (e.g. relay
binds but rejects all peers). That class is the observability stack's
job (#460); until it lands, post-deploy smoke checks are manual:
`curl https://willow.<domain>` + connect a known peer + verify message
delivery.

## Security baseline

A separate section because security touches every layer; collecting it
in one place makes audit easier than scattering it through 14 decision
sections.

**Account hardening (mandatory before Phase 2 cutover).**

- **Hetzner Cloud account.** Hardware-token 2FA (WebAuthn / FIDO2)
  required for every account holder. API tokens are scoped per
  purpose (read-only for status checks, write for OpenTofu); rotate
  on a 90-day cadence; tokens stored in agenix never on developer
  laptops. Email alerting on rescue-mode boot, SSH-key changes, and
  Storage Box auth failures.
- **Cloudflare account.** Hardware-token 2FA required. API tokens
  scoped to `Zone.DNS:Edit` on the willow zone only — never the
  account-wide token. CAA records pin Let's Encrypt as the only
  permitted issuer (with `accounturi=` constraint to a single ACME
  account). Cloudflare audit log enabled.
- **GitHub.** Hardware-token 2FA required for every collaborator with
  push or environment-approval rights. Branch protection on `main`:
  required PR review, required status checks, no force-push.
- **Storage Box.** SSH-key-only, separate keys for "host-write
  append-only" and "ops-prune full". Never accessed via password.

**Service hardening (per-unit defaults via `nix/modules/hardening.nix`).**

Every willow systemd unit (relay, replay, storage, restic, caddy)
inherits:

```nix
{
  serviceConfig = {
    NoNewPrivileges = true;
    ProtectSystem = "strict";
    ProtectHome = true;
    PrivateTmp = true;
    PrivateDevices = true;
    ProtectKernelTunables = true;
    ProtectKernelModules = true;
    ProtectKernelLogs = true;
    ProtectControlGroups = true;
    RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
    RestrictNamespaces = true;
    LockPersonality = true;
    MemoryDenyWriteExecute = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
    CapabilityBoundingSet = [ ];
    AmbientCapabilities = [ ];
    ReadWritePaths = [ /* per-service narrow list */ ];
  };
}
```

Verified per-unit with `systemd-analyze security <unit>` — target
score ≤ 3.0 ("OK"). Rust binaries don't JIT, so
`MemoryDenyWriteExecute` is safe; iroh's QUIC stack doesn't need
extra capabilities. Any deviation per unit is justified inline.

**OpenTofu state.** Stored in the Storage Box as a tfstate object
encrypted at rest using OpenTofu 1.7+'s
[state encryption](https://opentofu.org/docs/language/state/encryption/)
with a key from agenix. State contains plaintext API tokens for
Hetzner and Cloudflare in the default flow; encryption is required.

**Hetzner Volume LUKS.** The Volume is LUKS2-encrypted; key in agenix
(see §2). Recovery key offline (see §13).

**Per-developer break-glass SSH.** Distinct from the CI deploy key.
Each operator has a personal key in `authorized_keys` permitting
interactive sessions; key set is in `secrets.nix` and rekeyed when an
operator leaves. CI deploy key is *not* permitted interactive sessions
(forced-command), per §11.

**Audit + alerting glue.**

- `journalctl -u willow-relay` errors are captured locally; remote
  shipping deferred to #460. Until then, weekly `journalctl
  --since "1 week ago" --priority=warning` review is documented as a
  recurring operator task.
- Backup failures email out via the path described in §13.
- Hetzner panel and Cloudflare audit logs are reviewed monthly.

**Ownership, cadence, and verification.** "Required" without an owner
is aspirational. Each control above has a named owner and a verifier:

| Control | Owner | Cadence | Evidence captured |
|---|---|---|---|
| Hetzner / Cloudflare / GitHub 2FA | Account holder | At account creation, audited quarterly | Screenshot of 2FA settings page, attached to a quarterly `docs/reports/YYYY-QN-security-review.md` commit |
| API token rotation (Hetzner, Cloudflare, Cachix push, Storage Box) | Designated infra-on-call | 90 days | OpenTofu plan diff showing rotated token name + agenix commit log |
| `journalctl --priority=warning` review | Designated infra-on-call | Weekly | Findings noted in operator log; non-event reviews allowed to be silent but not skipped |
| Cloudflare + Hetzner audit-log review | Designated infra-on-call | Monthly | Brief note in operator log |
| Restore drill | Whole team | Every 6 months | Timed run, durations and any deviations captured in `docs/reports/` |
| `systemd-analyze security` ≤ 3.0 verification | CI | Every deploy | CI job fails on regression |

Until the project has multiple operators, the "designated infra-on-
call" is a single person; the rotation table becomes meaningful in
Phase 2+.

**Multi-approver requirement.** The Phase-1 manual-approval gate is
single-approver. A single compromised approver session — phishing-
resistant 2FA notwithstanding — still gives prod root via the
`production` GitHub environment. Once the team is ≥ 3 operators, the
environment is reconfigured to require **two distinct approvers**
(GitHub `required_reviewers ≥ 2` on the environment). Tracked in
"Future work"; until then, single-approver is the documented residual
risk.

**Action supply-chain bump checklist.** SHA-pinning third-party
GitHub Actions defends against tag retargeting, not against an action
repo whose SHA you adopt being compromised at adoption time. Every
dependabot bump for `cachix/install-nix-action`, `actions/checkout`,
`DeterminateSystems/nix-installer-action`, etc. is reviewed against
this checklist before merge:

1. Cross-reference the new SHA against the publisher's signed tag /
   release attestation (or, where available, the publisher's
   provenance attestations on GitHub).
2. Read the diff between current and proposed SHA. Trivial bumps
   should look trivial; if the diff is large or touches secret
   handling, escalate.
3. Prefer first-party Nix actions
   (`DeterminateSystems/nix-installer-action`) over community
   actions where the threat surface is comparable, because the
   publisher is more accountable in case of dispute.
4. Record the review outcome in the dependabot PR.

**ACME / CAA ordering.** Phase-1 implementation cannot create the
`accounturi`-constrained CAA record before Caddy has run, because the
ACME account URL is generated on first issuance. The bootstrap
sequence is:

1. Deploy Caddy with permissive CAA: `issue "letsencrypt.org"`.
2. Wait for first successful certificate issuance.
3. Extract Caddy's ACME account URL from
   `/var/lib/caddy/.local/share/caddy/acme/.../accounts/.../account.json`.
4. Tighten CAA via OpenTofu to include
   `accounturi="<account-url>"`.
5. Verify renewal still succeeds against the tightened CAA.

This sequence is included in the Phase-1 implementation runbook so
it isn't discovered the hard way at first renewal.

**Compromise response order.** When an operator-side compromise is
suspected (laptop loss, phished session, observed unauthorised
deploy), the canonical revocation order — designed so that earlier
steps don't break the operator's ability to do later steps — is:

1. Revoke the suspect operator's GitHub session(s) via the GitHub
   security log; require re-authentication.
2. Remove the operator's key from the GitHub `production`
   environment approver list (kills future deploys).
3. Rotate the GitHub Actions deploy SSH key on prod
   (`authorized_keys` rotated via deploy-rs from a known-clean
   operator).
4. Rotate the closure-signing key in `WILLOW_NIX_SIGNING_KEY` and
   redeploy with the new public key in `trusted-public-keys`.
5. Rotate Cachix push token and rebuild any cached paths from
   trusted source.
6. Rotate Hetzner and Cloudflare API tokens; commit the new
   ciphertexts via agenix from a known-clean operator.
7. If the operator's age private key is suspect, regenerate it,
   remove the old public key from `secrets.nix`, run
   `agenix --rekey` for every blob, commit and deploy.
8. Rotate the restic repository password and Storage Box append-only
   key (if exposure to backup system is suspected).
9. Rotate the LUKS volume key (the LUKS slot is replaced; data is
   unaffected).
10. Issue a security advisory under `docs/reports/`.

The order matters: rotating Hetzner API tokens (step 6) before
rotating the GitHub deploy key (step 3) would lock the operator out
of the `tofu apply` path needed to redeploy. The order is rehearsed
during the 6-monthly restore drill.

**Hetzner abuse-desk runbook.** Hetzner sends abuse notices to the
account email of record. That address is monitored by the
infra-on-call (same role as token rotation). Notices typically arrive
about port scans, copyright DMCA, or outbound spam — none of which
should originate from a relay-only workload, so any notice is treated
as an incident. SLA: respond within 4 hours of receipt; document
investigation outcome in `docs/reports/`. Hetzner's own escalation
window is 24 hours before service suspension; do not let an
unanswered notice expire.

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
│   │   └── willow-prod.nix       # CAX21 in FSN1, imports modules
│   └── disko/
│       └── willow-prod.nix       # disk + volume layout (CAX21 + Volume)
├── tofu/
│   ├── main.tf                # hcloud server, volume, firewall, FIP
│   ├── dns.tf                 # cloudflare records
│   ├── variables.tf
│   └── outputs.tf
└── secrets/
    ├── secrets.nix              # who-can-decrypt-what (recipient matrix)
    ├── restic-password.age      # host-decryptable (services.restic)
    ├── willow-volume-luks.age   # host-decryptable (boot-time LUKS unlock)
    ├── nix-signing-pubkey.txt   # plaintext; pinned in trusted-public-keys
    ├── tfstate-encryption.age   # operator-decryptable (OpenTofu state encryption)
    ├── smtp-credential.age      # host-decryptable (backup-failure alerts)
    ├── hcloud-token.age         # operator + CI; never deployed to host
    └── cloudflare-token.age     # operator + CI + host (DNS-01 ACME)

.github/workflows/
├── check.yml                  # PR gate (existing `just check-all`)
└── deploy.yml                 # main → build → cachix → deploy-rs

scripts/
└── (existing dev.sh stays untouched)
```

The `.claude/skills/deploy/SKILL.md` is rewritten as a thin wrapper:
"Run `nix run .#deploy -- .#willow-prod` after `tofu apply`. Secrets are
managed via agenix; see `infra/secrets/`."

**`secrets.nix` recipient matrix.** This file is the authority on
production access; the spec includes its skeleton here (illustrative;
real public-key values redacted):

```nix
let
  # Host identities (derived from /etc/ssh/ssh_host_ed25519_key.pub
  # via ssh-to-age, captured at first deploy)
  willow-prod      = "age1...prodhost...";

  # Operator identities (long-lived; ideally on YubiKey)
  alice            = "age1...alice...";
  bob              = "age1...bob...";

  # CI identity (GitHub-Secrets-resident; rotatable)
  ci               = "age1...ci...";

  operators = [ alice bob ];
  hosts     = [ willow-prod ];
in
{
  "restic-password.age"     .publicKeys = hosts ++ operators;
  "willow-volume-luks.age"  .publicKeys = hosts ++ operators;
  "smtp-credential.age"     .publicKeys = hosts;
  "tfstate-encryption.age"  .publicKeys = operators ++ [ ci ];
  "hcloud-token.age"        .publicKeys = operators ++ [ ci ];   # never host
  "cloudflare-token.age"    .publicKeys = hosts ++ operators ++ [ ci ];
}
```

Operators decrypt every secret needed for full DR (see §13). Host
decrypts only what it needs at runtime (no API tokens). CI decrypts
what it needs to drive `tofu apply` and signing — but **not** the
restic password, LUKS key, or SMTP credential, which never leave the
host or operator vault.

## Implementation phases

The migration is sequenced so each phase produces a working system and
leaves a clean rollback point.

### Phase 0 — Compromise response (day 1, same day as spec merge)

The root SSH password `WillowP2P2026deploy!` was committed in
`.claude/skills/deploy/SKILL.md`. **This password is permanently
public** — git history, GitHub clones, mirrors, archive scrapers, and
any AI training data pipeline that ingests public repos all retain
copies. Removing it from a future commit does not erase it.

Phase 0 treats the leak as a confirmed compromise and acts
accordingly. In order:

1. **Rotate the password** on the current Linode VM (or set a long
   random one) **before** anything else, *then* remove it from the
   skill file in a single commit.
2. **Disable password auth entirely** on Linode SSH:
   `PasswordAuthentication no` in `sshd_config`. Switch to keys-only
   immediately, with a temporary `ops` user.
3. **Audit auth logs** (`/var/log/auth.log`, `last`, `wtmp`) on the
   Linode VM for unauthorized SSH sessions during the leak window.
   Document findings. The audit conclusion is the input to the
   Phase-2 decision gate (whether to migrate the relay key); it is
   not used outside that gate.
4. **Treat any secret the Linode VM ever held as compromised**:
   relay/replay/storage Ed25519 keys, agent harness keys, any `.env`
   files. Plan their rotation as part of the cutover schedule.
5. **Publish a security advisory** in `SECURITY.md` (or a dated note
   under `docs/reports/`) documenting the leak window, exposed
   artifacts, rotation actions taken, and the Phase-0 audit outcome.
   This is **mandatory**, not optional, and is the user-visible
   responsible-disclosure record.
6. **(Optional, hygiene only)** A `git filter-repo` rewrite to remove
   the password from public history can follow. This **does not**
   count as remediation: the password is already public via clones,
   mirrors, and archives. Treat it as repo cleanup, not as
   compromise response, and only run after step 5's advisory is
   published.

Phase 0 is independent of the rest and reduces immediate risk while
the new stack is built.

### Phase 1 — Greenfield Hetzner box + backups (2–3 days)

- Stand up `infra/tofu` + `infra/nix` skeleton.
- Provision Hetzner CAX21 + LUKS Volume + Firewall + Cloudflare records
  under a staging hostname (e.g. `willow-staging.<domain>`).
- Run `nixos-anywhere` once with the bootstrap-minimal config; capture
  the host's age public key; run `agenix --rekey`; redeploy with the
  full configuration. (See §8 bootstrap dance and Appendix C runbook.)
- Deploy relay/replay/storage as fresh peers with new identities on
  the staging host.
- **Provision Hetzner Storage Box and enable nightly `restic` backups
  before cutover.** This was originally Phase 3; reordered so that
  the new prod is never live with state but no backups.
- **Run the documented restore drill on the staging host** — destroy
  the VM, recreate it via OpenTofu, run `nixos-anywhere`, restore
  `/etc/willow` + `/var/lib/willow` from `restic` using the offline
  recovery secrets, verify the storage SQLite is byte-identical to
  pre-destroy. This is the spec's hardest verification gate — Phase 2
  cannot start until it passes (per
  `superpowers:verification-before-completion`).
- Verify Caddy + TLS on the staging hostname, run a peer end-to-end
  via the agent harness against the staging relay.

Outcome: production parity at the staging hostname *with verified
backups and a verified restore path*; production (Linode) still
serving real users.

### Phase 2 — Cutover (≤ 1 hour, scheduled window)

- **Decision gate.** If the Phase-0 audit ruled out unauthorized
  access during the password-leak window, the relay's Ed25519 key is
  migrated (peer-ID continuity). If not, the relay rotates to a fresh
  key and existing peers re-discover via DNS. Record the decision in
  the cutover commit message.
- **Migrate the relay's Ed25519 key** per Appendix C runbook: stream
  through `age` end-to-end, never write to the laptop disk, integrity-
  check the result by comparing the relay's advertised peer ID
  against the Linode value before DNS cutover, and run `shred` on
  the source `/etc/willow/relay.key` immediately before powering
  off the Linode VM. Workers (`replay`, `storage`) get fresh
  identities on Hetzner; SyncProvider permission is re-granted via a
  single state event after they connect.
- **Re-point DNS in Cloudflare** to the new IP. Pre-stage with low
  TTL (60s) the day before so propagation is effectively instant.
- **Single-key invariant** (not full atomicity). Only one host runs
  the relay process with the live key at any moment. Concretely:
  stop the Linode relay; verify it is stopped; copy the key; start
  the Hetzner relay; verify it advertises the expected peer ID; cut
  DNS. Failing any step, abort and roll back.
- **Acknowledged reachability gap.** Between "start Hetzner relay"
  and "DNS TTL expires" there is a window (≤ 60s with TTL 60) where
  DNS still points at Linode but the Linode relay is stopped. Peers
  that hit Linode in this window get TCP RST / connection-refused.
  Clients with exponential backoff (the default for `willow-client`)
  recover transparently; clients without retry logic see a sub-2-minute
  outage. Accepted as the cost of the single-key invariant; documented
  here so the cutover note can capture observed user impact.
- Decommission Linode VM after a 7-day soak (this is what actually
  deallocates the underlying storage; see Appendix C step 11).

### Phase 3 — CI deploy path with manual approval (1 day)

- Add `.github/workflows/deploy.yml` with a `production` environment
  that requires manual approval.
- Implement closure signing: deploy artefacts signed with a Nix key
  in agenix; production's `trusted-public-keys` lists only that key.
- Pin all third-party actions by SHA, set per-workflow `permissions:`
  to minimum.
- Move from "developer laptop runs `deploy-rs`" to "merge to `main`
  → CI builds + signs → reviewer approves → CI ships".
- Keep the laptop path working as a break-glass procedure (per-
  developer key, separate from CI key).

### Phase 4 — Observability (deferred, issue #460)

- Track separately. Targets the same NixOS modules and reuses `agenix`
  for monitoring secrets.

## Cost summary (steady state)

Prices reflect Hetzner's 2026-04-01 adjustment.

| Item | EUR/mo |
|---|---|
| Hetzner CAX21 ARM (compute) | 7.99 |
| Hetzner Volume (10 GB, LUKS-encrypted) | 0.57 |
| Hetzner Primary IPv4 | 0.50 |
| Hetzner Storage Box BX11 (1 TB) | 3.20 |
| Cloudflare DNS + optional proxy | 0.00 |
| Cachix (free OSS tier, 5 GB) | 0.00 |
| GitHub Actions (public repo) | 0.00 |
| **Total** | **~12.26** |

Under the €20/mo goal with ~€8/mo headroom. Switching to a CPX21 (AMD,
3 vCPU, 4 GB) would raise the line item from €7.99 to €11.99 (CAX21 wins
on $/perf for our workload, see §1).

A Hetzner **Floating IP** (€3.00/mo, *not* the Primary IP at €0.50) is
not in the Phase-1 budget. It becomes worthwhile during the multi-region
or DDoS-scrubbing rollout because it can be reassigned between hosts in
seconds without DNS propagation.

Phase-2 multi-region adds another full host: ~€9/mo for compute
+ Volume + Primary IP (Storage Box is shared so excluded), and a
Floating IP per region if used (€3/mo).

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
  SyncProvider. Floating IP per region for fast cutover.
- **Self-hosted Nix binary cache** via [`celler`](https://discourse.nixos.org/t/celler-an-attic-fork/77265)
  if Cachix free tier is exhausted or its terms change. Note: `attic`
  upstream is stalled; the actively-maintained route is the celler
  fork.
- **Migration to `sops-nix` + hardware-token age identities** if the
  agenix host-key threat model becomes uncomfortable (see §8). For
  high-value secrets that cannot be rotated at source.
- **Relay key rotation cadence.** The relay's Ed25519 key, migrated
  once at cutover, becomes a long-lived secret. Rotation requires
  prior announcement and a peer-discovery overlap window where both
  keys serve. Annual cadence proposed; ties into Willow's outbox-
  relay-discovery work (`docs/specs/2026-04-24-outbox-relay-discovery.md`).
- **Observability stack** — see issue #460.
- **DDoS scrubbing for relay TCP/9090.** A volumetric attack on the
  relay is currently unmitigated by anything beyond per-source
  nftables rate limits (see §14). **Trigger:** sustained > 100 Mbps
  inbound on TCP/9090, or relay reachability < 95% from a third-party
  external probe over a 10-minute window.
  **Fallback runbook:** reassign the Hetzner Floating IP to a
  temporary scrubbing host (a second CAX21 in a different DC running
  `iptables -m hashlimit` + iroh proxy), then triage upstream. Decide
  between (a) relay-of-relays topology, (b) upstream-tunnel-via-QUIC
  scrubbing service, or (c) accept the residual risk based on observed
  attack patterns.
- **Off-Hetzner restic mirror.** `restic` snapshots live entirely
  inside Hetzner's network today — Storage Box reachability depends
  on Hetzner being cooperative (not legally holding the account, not
  applying an AUP suspension during a dispute). A weekly off-Hetzner
  mirror to Backblaze B2 with separately-managed credentials removes
  the single-point-of-recovery-failure. Phase-2 work; cost ~€1/mo
  for ~100 GB at B2 list rate.
- **Two-of-N approver requirement on `production` GitHub
  environment.** Phase-1 manual-approval is single-approver; once
  team is ≥ 3 operators, raise `required_reviewers ≥ 2` so a single
  compromised approver session cannot ship to prod.
- **Disaster-recovery game day.** Schedule a periodic exercise where
  the prod VM is intentionally destroyed and rebuilt from `infra/` +
  restic + offline secrets; measure recovery time. Target: <30
  minutes.
- **Hardware key for personal age identities** (`age-plugin-yubikey`)
  for operators who hold break-glass / approver permissions.
- **OpenTofu state encryption** verification under the 1.7+ encryption
  feature; planned in implementation but not yet exercised.
- **`nh` adoption** when its rollback story matures to deploy-rs
  parity.

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

## Appendix C: Relay key migration runbook

This is the procedure invoked during Phase 2 cutover. Every step is
written to be re-readable in a stressful 1am window. Deviation is not
a feature; if the runbook is wrong, fix the runbook and re-run.

**Pre-conditions**

- Phase 0 auth-log audit passed; key migration has been authorised
  (Phase 2 decision gate).
- Hetzner host is fully provisioned, configured, and idle (relay
  process not yet started, or started with a throwaway test key).
- Operator has SSH access to both Linode and Hetzner via personal key.
- Operator has the Hetzner host's age public key in
  `./hetzner-host-age.pub`, derived from
  `/etc/ssh/ssh_host_ed25519_key.pub` via `ssh-to-age`.
- All `ssh` invocations below use `-o ForwardAgent=no` (set globally
  via `~/.ssh/config` for the duration of the runbook). **Never** use
  `ssh -A` against Linode while it is treated as potentially
  compromised.

**Operator shell preamble (paste at top of session, before step 1)**

```sh
set -euo pipefail              # any failure aborts the session
unset HISTFILE                 # current shell history → /dev/null
ssh() { command ssh -o ForwardAgent=no "$@"; }
[[ -s hetzner-host-age.pub ]]  # bail if recipient file empty/missing
```

(For zsh/fish, the local-`unset HISTFILE` equivalent is
`HISTFILE=/dev/null` for zsh, `set -gx HISTFILE /dev/null` for fish.
The `set -euo pipefail` is bash; portable equivalents per shell.)

To suppress history on the *remote* hosts, every multi-line remote
block runs as `ssh <host> 'HISTFILE=/dev/null bash -c "..."'`.

**Steps**

1. **Stop the Linode relay** and verify it has stopped:

   ```sh
   ssh linode 'HISTFILE=/dev/null bash -c "systemctl stop willow-relay && systemctl is-active willow-relay"'
   ```

   Expect `inactive`. Do not proceed otherwise.

2. **Record the source's peer ID and key hash.** Peer ID is the
   *security-relevant* identity property (a substituted key would
   produce a different peer ID); the SHA-256 is a transport-correctness
   check.

   ```sh
   SOURCE_PEER_ID=$(ssh linode 'HISTFILE=/dev/null willow-relay --print-peer-id --identity-path /etc/willow/relay.key')
   SOURCE_HASH=$(ssh linode 'HISTFILE=/dev/null sha256sum /etc/willow/relay.key' | awk '{print $1}')
   echo "Linode peer ID: $SOURCE_PEER_ID"
   echo "Linode key hash: $SOURCE_HASH"
   ```

   Record both in the cutover note. If either looks unexpected, abort
   and treat as a Phase-0 compromise extension.

3. **Stream the key end-to-end through `age`**, never writing
   plaintext to the operator's laptop:

   ```sh
   ssh linode 'HISTFILE=/dev/null cat /etc/willow/relay.key' \
     | age -r "$(cat hetzner-host-age.pub)" \
     | ssh hetzner 'HISTFILE=/dev/null bash -c "age -d -i /etc/ssh/ssh_host_ed25519_key > /etc/willow/relay.key.new && chmod 0400 /etc/willow/relay.key.new && chown willow-relay:willow-relay /etc/willow/relay.key.new"'
   ```

4. **Verify transport integrity (hash match):**

   ```sh
   DEST_HASH=$(ssh hetzner 'HISTFILE=/dev/null sha256sum /etc/willow/relay.key.new' | awk '{print $1}')
   [[ "$DEST_HASH" == "$SOURCE_HASH" ]] || { echo "HASH MISMATCH"; exit 1; }
   ```

5. **Swap into place** on the Hetzner host:

   ```sh
   ssh hetzner 'HISTFILE=/dev/null mv /etc/willow/relay.key.new /etc/willow/relay.key'
   ```

6. **Start the Hetzner relay and verify peer-ID continuity** —
   the security-critical check:

   ```sh
   ssh hetzner 'HISTFILE=/dev/null systemctl start willow-relay'
   DEST_PEER_ID=$(ssh hetzner 'HISTFILE=/dev/null willow-relay --print-peer-id --identity-path /etc/willow/relay.key')
   [[ "$DEST_PEER_ID" == "$SOURCE_PEER_ID" ]] || { echo "PEER ID MISMATCH"; exit 1; }
   ```

   A peer-ID mismatch here means the migration produced a different
   advertised identity than the Linode source — could be transport
   corruption (would also fail step 4), substitution at the source
   (Linode compromise), or operator error. Abort.

7. **Cut DNS in Cloudflare** to point `relay.<domain>` and (if
   applicable) `willow.<domain>` at the Hetzner IP. Cloudflare
   propagation completes in seconds with TTL 60.

8. **Smoke test** from a fresh peer (e.g. `willow-agent` from a
   developer laptop) to confirm gossip + WSS work end-to-end against
   the new relay's peer ID.

9. **Remove (not "shred") the source key** before powering off Linode.
   `shred` is unreliable on virtualized block devices and Hetzner /
   Linode network volumes — overwrite-in-place semantics are not
   guaranteed. Honest framing: the destination of last resort is
   destroying the Linode VM (step 11), which deallocates the
   underlying storage. Removal here is best-effort hygiene:

   ```sh
   ssh linode 'HISTFILE=/dev/null rm -f /etc/willow/relay.key /root/.bash_history /home/*/.bash_history'
   ```

10. **Power off the Linode VM** via the Linode panel (not
    ssh-on-the-box, so the panel-level audit log captures the
    operator action). Do not destroy yet — keep for the 7-day soak.

11. **Revoke operator SSH keys on Linode** at the panel level
    so re-power-on doesn't restore SSH reachability without
    re-authorisation. After the 7-day soak, **destroy the Linode VM**
    via the panel — only this step deallocates the underlying storage
    and provides the actual key-disposal guarantee that step 9 cannot.

12. **Record the cutover** in a commit on `main`:
    `chore(infra): cut over to Hetzner; relay peer ID preserved`,
    citing the recorded `$SOURCE_PEER_ID` and the Phase-0 audit
    conclusion.

**Failure handling.** If any verification step (2 sanity, 4 hash, 6
peer ID) mismatches, or if step 8's smoke test fails: abort, keep
Linode powered off, revert DNS, restore from `restic` if Hetzner
state was modified, and debug offline. The `set -euo pipefail`
preamble means a `[[ ... ]]` mismatch terminates the session — do
**not** improvise around it.

**What this runbook deliberately does not do**: scp the key through
the operator's laptop, base64-encode-and-paste it through a clipboard,
forward the operator's ssh-agent to a potentially compromised host,
rely on `shred` semantics that the underlying storage doesn't honour,
or trust the operator's memory of any value longer than a hash.
