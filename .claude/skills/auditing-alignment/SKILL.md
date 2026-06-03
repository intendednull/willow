---
name: auditing-alignment
description: Use when a spec is finished and implementation is about to start, or when a feature implementation is complete and you need to reconcile the as-built code with the spec. Surfaces misalignments between spec, plan, and code, then classifies each one as "update the spec" or "update the code." Reporting "all clear" with no findings is a valid and good outcome.
---

# Auditing Alignment

## Overview

The Willow workflow is **spec → plan → code**. Misalignments accumulate at every step:

- Specs make assumptions about the codebase that turn out to be wrong.
- Implementers make decisions during coding that diverge from the spec — sometimes for good reasons, sometimes by accident.
- Plans evolve. Code evolves. Specs stagnate.

This skill is a **focused alignment audit** at one of two checkpoints:

- **Pre-implementation**: spec exists, no/early plan, no code yet. Catch shape conflicts between spec and codebase before they cost real time.
- **Post-implementation**: feature is "done." Reconcile spec ↔ code. For each divergence, decide whether the spec or the code should be updated.

**Finding nothing is fine.** "All clear" is a valid outcome. Padded reports are worse than empty ones.

## When to Use

| Situation | Mode |
|---|---|
| Spec just written, no plan/code yet | Pre-implementation |
| Plan exists, code not yet written | Pre-implementation |
| Implementer reports feature complete | Post-implementation |
| Suspect drift between spec and shipped code | Post-implementation |

**Do NOT use** for general code review, security sweeps, or tech-debt audits. Different scope — see `general-audit`.

## The Decision Tree

For every misalignment found, classify it. The classification determines the proposed fix. Always **propose to the human, then act** — never silently edit spec or code.

```
Misalignment between SPEC and CODE?
│
├─ Code does not implement something the spec promises
│   ├─ Still desired                         → propose: implement in code
│   ├─ Deliberately dropped during impl      → propose: update spec (remove/mark as deferred)
│   └─ Now obsolete                          → propose: update spec (remove)
│
├─ Code does something the spec does not describe
│   ├─ Implementer's approach is better      → propose: update spec to match
│   ├─ Out-of-scope addition / scope creep   → propose: revert/simplify code
│   └─ Implementer filled an obvious gap     → propose: update spec to document
│
├─ Spec contradicts codebase shape/conventions (pre-implementation only)
│   ├─ Codebase shape is the right shape     → propose: update spec
│   └─ Spec is right, code needs to change   → propose: refactor task (separate work)
│
└─ Plan diverges from spec
    ├─ Substantive shift not captured by spec → propose: update spec or revise plan
    └─ Minor tactical evolution               → ignore — plans are allowed to evolve

No misalignments → record "all clear" and stop.
```

## Pre-Implementation Workflow

Goal: catch shape conflicts before code is written.

1. **Read the spec end-to-end.** No skimming.
2. **List the integration points.** Every place the spec says "this attaches to existing X" or "extends Y." For each:
   - Find the actual file/module in the codebase.
   - Verify the API/contract the spec assumes still exists.
   - Verify there isn't already a different abstraction the spec missed (e.g. spec says "add a Mutex" but the codebase uses actors).
3. **List the shape claims.** Every architectural assumption the spec makes about the codebase:
   - Async model (blocking vs async, native vs WASM constraints)
   - State management (lock vs actor — see CLAUDE.md table)
   - Crate boundaries
   - Test tier the spec implies
   - Trait abstractions the spec assumes vs. what exists
4. **For each conflict, classify** via the decision tree.
5. **Report.**

## Post-Implementation Workflow

Goal: reconcile as-built code with the spec.

1. **Re-read the spec.** Extract a flat list of behaviors / features / contracts it promises.
2. **Spec → Code pass.** For each spec promise, find where the code delivers it. Missing? Note.
3. **Code → Spec pass.** Walk the implementation diff (or the relevant modules if no clean diff). For each substantial decision (new public API, new state, new event kind, new permission, new test tier, new dep), find where the spec describes it. Not described? Note.
4. **Read the plan _after_ steps 1–3.** Plans bias comparison if read first. If the plan documents the divergence with a reason, the divergence is intentional — confirm the spec captures the new state.
5. **Classify each finding** via the decision tree.
6. **Report findings + recommendations.**

## All-Clear Is Good

Do not pad reports. A valid all-clear report is short:

> Audited alignment between `docs/specs/2026-04-26-foo-design.md` and code under `crates/foo/`.
> No misalignments found. Spec and code are in sync.

If only one or two findings exist, that's the report. Don't reach for more.

## Reporting Format

For each finding:

```
**Finding**: <one-line summary>
**Spec**:    <docs/specs/...:section> or "(spec silent)"
**Code**:    <crate/path/file.rs:line> or "(code missing)"
**Type**:    code-update | spec-update | refactor | discussion
**Why**:     <one or two sentences>
**Proposed**: <concrete change>
```

Group findings by type. Lead with a one-line summary:

> `N code-update, N spec-update, N refactor, N discussion. M total.`

If `M == 0`: report "all clear" instead.

## Lessons-Learned Loop

At the end of every run, **ask the human** before editing this skill file. Do not push edits unsolicited.

Prompt format:

> Run complete. <one-line summary of findings>.
> Anything we learned this run that should improve future iterations of this skill?
> If yes, I can propose specific edits to `.claude/skills/auditing-alignment/SKILL.md`.

If the human says yes:

1. Draft each lesson as a concrete edit (added section, new heuristic in the decision tree, new red flag, refined trigger).
2. Show the diff (use `Edit` previews or describe the change inline) — do not apply yet.
3. Apply only the edits the human approves. Skip the rest.
4. If multiple lessons feel related, propose them as one coherent edit, not a flurry of small ones.

If the human says no, accept it — no edit, no follow-up nudge.

## Hard Rules

- **"All clear" is a valid outcome.** Do not invent findings to look productive.
- **One spec per audit.** Don't sweep the whole codebase. Stay scoped to the feature in question.
- **Recommend, don't act.** Code and spec edits need human approval. The only exception is the lessons-learned edit to this skill file, and only after the human says yes.
- **Read the plan after spec + code**, not before. Reading the plan first biases the comparison toward what the implementer happened to do.
- **Don't audit security, performance, or tech debt here.** Different scope. Different skill.

## Red Flags

| Thought | Reality |
|---|---|
| "I should find at least one thing" | No. All-clear is the goal when nothing's wrong. |
| "Let me audit security/perf while I'm here" | No. Stay scoped. |
| "I'll just fix this one small misalignment" | No. Propose, ask, then act. |
| "The spec is obviously right" | Maybe not. Code may have evolved past it for good reason. |
| "The code is obviously right" | Maybe not. Spec may capture a constraint the code missed. |
| "I learned a useful lesson — let me update the skill" | Ask the human first. Then apply. |
| "The plan is the source of truth" | No. Spec is the source of truth for intent. Plan is tactical and allowed to evolve. |

## Common Mistakes

- **Reading the plan first.** Biases the spec ↔ code comparison toward what the implementer did. Always: spec → code → plan.
- **Auditing scope creep _outside_ the feature.** If the implementer touched unrelated code, that's a separate concern; flag it but don't expand the audit into a general review.
- **Treating spec as immutable.** It isn't. Implementer decisions can be better than spec — the skill exists in part to surface those and update the spec.
- **Treating code as immutable.** It isn't either. Code can drift from intent. Sometimes the right outcome is a refactor task.
- **Editing the skill file without asking.** Lessons learned must be human-approved. Always.
