---
name: general-audit
description: Use when running a scheduled general audit of the Willow codebase, or when /general-audit is invoked on a pull request for review
user-invocable: true
---

# General Audit

You = master orchestrator. Fresh agents do all work. Job = find + file findings. Resolution = separate routine.

## When to Use

- Scheduled run on `main`: full-tree audit, files findings as issues.
- `/general-audit` invoked in a PR: review the PR only — no issues filed.

## Core Task

Audit full codebase, main branch only. Skip if HEAD == commit in last report.

Spawn parallel agents, narrow by concern (not file scope). Default split:
- security → sub-split: input validation/DoS, auth/permissions, web/WASM, deps/supply-chain
- tech debt / code quality
- clean architecture (diff specs vs code; pass spec paths explicitly)
- test coverage
- general review

Spawn more if area needs depth.

## Synthesis

Collect findings → master issue (commit + all findings) + child issue per finding. Cross-ref open issues here for dedup. Second pass w/ fresh agents: verify findings real + non-dup via grep/rg for exact patterns cited.

## Lessons Learned

Append "lessons learned" section to report. Feed back into this skill next run.

## /general-audit in PR

Same flow but review PR only. No issues filed.

## Hard Rules

### Scope
- Audit full tree always. Never scope to diff.
- Agents blind to existing issues. Dedup = synthesis + 2nd pass only.
- File findings only. No PRs, no auto-fix, no closing existing issues. Resolution = separate routine.

### Agent prompts (mandatory fields)
- Time budget: 6 min, stop+save if exceeded.
- Incremental write: scaffold report file before 2nd tool call; append each finding complete before next.
- Per-finding: file:line, severity (split: security = confidentiality/integrity; robustness = availability/DoS), Obvious? yes/no.
- Count/ratio claims: verify w/ second grep cmd proving count.
- Use general-purpose agent (Explore can't Write).
- Architecture agents: skip cargo tree/cargo clippy; use rg + ls + reads.
- GitHub comms (issues, comments, reviews) written in caveman mode. Code blocks + security warnings stay normal.

### Setup
- `cargo install --locked cargo-audit` upfront (or verify); run as 1st step in security/deps.

### Quality
- Quality > speed. Always thorough path.
- Independently spot-check every filed finding.
