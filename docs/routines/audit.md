# Audit

Master-orchestrator codebase audit. Runs in Claude web on a schedule, attached to a repo. Spawns parallel fresh agents to review the full tree, files findings as GitHub issues, and opens auto-fix PRs for obvious issues.

- **Where it runs:** Claude web, scheduled task, attached to a single repo at a time.
- **Cadence:** as scheduled in Claude web.
- **Output:** master report issue + child issues per finding + draft/auto-fix PRs.

## Prompt

Per repo attached. You = master orchestrator. Fresh agents do all work. Master report stays in own repo. No cross-repo mentions in individual reports.

### Core task

Audit full codebase, main branch only. Skip if HEAD == commit in last report.

Spawn parallel agents, narrow by concern (not file scope). Default split:
- security → sub-split: input validation/DoS, auth/permissions, web/WASM, deps/supply-chain
- tech debt / code quality
- clean architecture (diff specs vs code; pass spec paths explicitly)
- test coverage
- general review

Spawn more if area needs depth.

### Synthesis

Collect findings → master issue (commit + all findings) + child issue per finding. Cross-ref open issues here for dedup. Second pass w/ fresh agents: verify findings real + non-dup via grep/rg for exact patterns cited.

### Auto-fix

Obvious findings → open PR via git worktrees (parallel). Monitor CI till green. Ambiguous findings → draft PR w/ questions in description.

### Background

Fresh agent sweeps existing open issues for resolved/false-positive → close w/ reason comment. Conservative; no-op fine.

Identify other existing issues workable in parallel; same PR rules.

### Lessons section

Append "lessons learned" section to report. Feed back into this prompt next run.

### /audit in PR

Same flow but review PR only. No issues, no PRs.

---

## Hard Rules

### Scope
- Audit full tree always. Never scope to diff.
- Agents blind to existing issues. Dedup = synthesis + 2nd pass only.

### Agent prompts (mandatory fields)
- Time budget: 6 min, stop+save if exceeded.
- Incremental write: scaffold report file before 2nd tool call; append each finding complete before next.
- Per-finding: file:line, severity (split: security = confidentiality/integrity; robustness = availability/DoS), Obvious? yes/no.
- Count/ratio claims: verify w/ second grep cmd proving count.
- Use general-purpose agent (Explore can't Write).
- Architecture agents: skip cargo tree/cargo clippy; use rg + ls + reads.
- GitHub comms (issues, PRs, comments, reviews) written in caveman mode. Code blocks + security warnings stay normal.

### Setup
- `cargo install --locked cargo-audit` upfront (or verify); run as 1st step in security/deps.
- Pre-worktree: `git stash` or `git restore` main dir; add `.claude/worktrees/` to `.gitignore`.

### Quality
- Quality > speed. Always thorough path.
- Independently spot-check every filed finding.
