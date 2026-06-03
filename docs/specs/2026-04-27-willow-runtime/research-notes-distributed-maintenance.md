# Research notes — distributed maintenance and participation

**Date:** 2026-04-27
**Status:** notes for a future session, not a spec
**Parent:** [README.md](README.md)

## Reframe captured here, not in the master spec yet

Maintenance work for an app is not a separate runtime concept; it is
**a fourth class of components** alongside state / interaction / behavior:

- **Maintenance components** — persister, snapshot provider, sync provider,
  replay buffer. Optional in any app's bundle. Loaded by peers that opt
  to contribute, with kernel-known capacity hints.

A peer's "participation" is just the set of maintenance components it has
instantiated and the capacity it has declared for them. Scaling = more
peers running an app → more maintenance-component instances → more
maintenance capacity, automatically. There is no separate work-tracking
subsystem; the runtime model already supports this.

Default client behavior: opt-in to cheap maintenance roles by default,
expose a "how much do you want to contribute" UI for expensive ones
(disk-heavy persister, bandwidth-heavy sync provider).

The master spec should grow a section reflecting this when the next
session draft lands. **Do not draft it yet** — the participation /
free-rider problem below is load-bearing for what that section will
commit to, and it's research-heavy.

## The hard problem: participation enforcement under Sybil

A custom client that does not run maintenance components, multiplied by
spinning up many identities, free-rides on honest participants. The
honest peers' load grows with the cheaters' identity count.

User's first-cut proposal:

- Self-reported participation is gameable — can't be the input.
- Community-tracked participation: peer A observes peer B's contributions
  to A during interaction. Local view, gossip-aggregated.
- Refusal to serve non-participants becomes the enforcement primitive.
- Sybil resistance is the hard part — without identity cost, the metric
  is gameable by minting more identities.

This is the right family of solutions; the literature has 20+ years of
specific designs. The next session should start from prior art.

## Research topics for the next session

### A. Free-rider quantification in real P2P systems

- **Adar & Huberman, "Free Riding on Gnutella" (2000)** — the canonical
  measurement paper. ~70% of Gnutella users contributed nothing.
  Establishes the problem.
- **Hughes, Coulson, Walkerdine (2005)** on Gnutella free-riding evolution.
- BitTorrent measurement studies — why tit-for-tat reduced but did not
  eliminate free-riding.

### B. Tit-for-tat and reciprocity (no global identity needed)

- **BitTorrent's choking algorithm (Cohen 2003)** — local pairwise
  reciprocity. The most successful deployed answer. Sybil-tolerant
  because it's per-connection: cheating identities each get nothing
  individually until they upload.
- **PropShare** and other BitTorrent variants — refinements with
  formal analysis.
- Limitation: works for symmetric workloads (you have what I want, I
  have what you want). Maintenance work isn't symmetric — a snapshot
  provider isn't asking the joiner for anything in return. Pure tit-for-tat
  doesn't fit cleanly.

### C. Reputation and trust aggregation

- **EigenTrust (Kamvar, Schlosser, Garcia-Molina 2003)** — global trust
  scores via gossip eigenvector. Sybil-vulnerable but the canonical
  reference. Many later systems build on it.
- **PowerTrust, PeerTrust** — variants. Some Sybil-hardening.
- **Tribler's BarterCast** — local-view reputation in a real deployed
  P2P system. Practical lessons about gossip-aggregated reputation.

### D. BAR (Byzantine, Altruistic, Rational) game-theoretic models

- **BAR Gossip (Aiyer, Alvisi, Clement, Cowling, Dahlin, Riché 2005)** —
  framework for protocols that work even when some peers are Byzantine
  and others are merely rational (selfish). Directly relevant to "honest
  peers + free-riders + actively-malicious peers."
- **FlightPath, BAR Fault Tolerance** — followups.
- BAR is the right academic frame for our problem.

### E. Sybil resistance without proof-of-work

- **Castro, Druschel, Ganesh, Rowstron, Wallach, "Secure routing for
  structured P2P overlays" (2002)** — early Sybil mitigations.
- **SybilGuard, SybilLimit (Yu et al. 2006, 2008)** — social-graph-based
  Sybil resistance. Relevant if we're willing to use trust links between
  identities (who-trusts-whom data Willow already has via the existing
  permission/invite model).
- **Whanau (Lesniewski-Laas 2010)** — Sybil-proof DHT. Cleaner technique.
- Identity-cost approaches (proof-of-stake, proof-of-storage) — likely
  too heavy for chat-shape apps; flag as out-of-scope unless we change
  our minds.

### F. Storage proofs (relevant for persister role)

- **Filecoin proofs of replication / proofs of spacetime** —
  cryptographically verifiable storage. Heavy machinery; only relevant
  if we want strong durability guarantees.
- **Storj, Sia** — practical storage-proof systems.
- **Audit-style schemes** (challenge-response over stored data) — much
  cheaper. May be the right level for our case.

### G. Holochain's validator-selection / DHT-responsibility model

- **Holochain RFCs on validation responsibility** — every node validates
  entries it's "responsible for" in the DHT. Coordinated allocation
  (Pattern B in our earlier discussion). Closest existing system to
  what we're building, structurally.
- Worth a real read; lessons probably translate directly.

### H. IPFS / libp2p

- **Bitswap protocol** — ledger-based reciprocity for block exchange.
  Closer in spirit to BitTorrent than to a reputation system.
- **IPFS pinning economics** (Filecoin, Pinata, Web3.Storage) — what
  happens when a popular thing falls out of cache? Lessons about
  voluntary maintenance failure modes.

## What the next session should produce

1. A read-through of (B), (D), (E), and (G) — those are the closest fits.
2. A decision on which model Willow adopts: tit-for-tat (Bitswap-ish),
   reputation (EigenTrust-ish), social-graph Sybil resistance
   (SybilGuard-ish), or DHT-responsibility (Holochain-ish). Likely a
   hybrid.
3. An explicit decision on whether we use the existing permission/invite
   trust graph as the social-graph input — this is a unique advantage
   we have over generic P2P systems and may simplify Sybil resistance
   substantially.
4. A draft section for the master spec naming maintenance components
   and the participation primitive at master level, deferring the
   protocol details to a child spec.

## Notes captured for context

- The runtime model already supports maintenance-as-component without
  any kernel changes. The participation question is *not* about whether
  the runtime can express it; it's about what the protocol layer that
  enforces "refuse to serve non-participants" looks like.
- The existing permission/invite system means we already have a
  Sybil-relevant trust graph for free; this is genuinely an advantage
  over BitTorrent / IPFS / Gnutella, which had to bootstrap social
  graphs they didn't have. Worth surfacing in the next-session decision.
- Free-rider tolerance for chat-shape apps is high — most users contribute
  little, the cheap roles are nearly free, and the only adversary that
  matters is an automated client doing it at scale. This narrows the
  threat model usefully.
