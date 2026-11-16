# ISSUE-002 — Skill-install documentation is stale and triple-stated after harness plan 012: `AGENTS.md` §Skill Installation still calls `~/.agents` a "personal skills repository" with a cp loop whose 13-name list omits `knot-lint`, while `knot-init` v4.14.0 step 4a carries a divergent 14-name copy — the script (`scripts/deploy-skills.sh`) is now the act

**Status:** OPEN — raised 2026-11-16. **Blocked-on:** the in-flight uncommitted `AGENTS.md` edits (K6-discipline paragraph) must land first; pathspec discipline forbids carrying that strand's work.
**Raised:** 2026-11-16
**Raised by:** the harness session executing superproject plan 012 (ISSUE-007, `~/.agents` → deployment target), recorded as plan 012's deferred follow-up (harness `project/issues/issue-007-host-agents-deploy-topology.md`, act log).
**Landing target:** `AGENTS.md` §Skill Installation (rewrite) + `knot-init` v4.14.0 step 4a (re-point at the script, minor bump) — the rectification *removes* restatements, it does not add one.
**Severity:** medium — no runtime cost today (the script globs `knot-*` and the host is correct), but the documented act is now wrong in kind, the name lists have already drifted exactly the way `knot-lint` K6 exists to catch, and a follower of the doc verbatim silently under-deploys.

---

## 1. What changed underneath (harness plan 012, 2026-11-16)

- `~/.agents` is a **deployment target, not a repo** (de-gitted; history
  archived at `~/.harness-012-backup-2026-11-16/`). The phrase "personal
  skills repository" is dead.
- `scripts/deploy-skills.sh` is the deploy act: file act only, globbed
  (`knot*`, no hand-maintained name list), namespace-scoped prune
  (retired skills disappear — the cp-era `cp` never deleted and left
  stale residue on the harness host, found first-hand), `--status`
  dry run, `--committed` deploys HEAD instead of the working tree.

## 2. The defect (measured 2026-11-16)

Three restatements of one act, no two alike:

| Restatement | State |
|---|---|
| `AGENTS.md` deploy cp loop | **13 names — omits `knot-lint`**; `mkdir -p` + `cp -r`; no prune; the "personal skills repository" framing |
| `AGENTS.md` verify loop | 13 names again, plus hand-rolled diff verification |
| `knot-init` step 4a | **14 names — includes `knot-lint`**; its own cp + verify loops, "idempotent" claim without prune |

The in-flight `AGENTS.md` edits declare §Skill Installation "the single
canonical copy of the deploy/verify loop" — the canonical copy is the
stale one. `knot-init`'s list diverging from `AGENTS.md`'s is the
restatement-drift class the **K6 docs-drift check family** (v0.2.0)
exists to flag: K6 already checks "deploy-loop copies"; a check that a
documented name-list matches the script's glob (or: that no deploy-loop
copy exists outside the script) is a natural K6 extension — **candidate,
own decision when the rewrite lands** (owner ruling 2026-11-13: rig
neutrality — docs in this repo may name the script, nothing outside).

## 3. Rectification shape (the rewrite removes, never restates)

1. **`AGENTS.md` §Skill Installation** → short section: skills are
   developed at `.agents/skills/` (tracked here, the knot skills'
   **home** — harness plan 012); deployed by `scripts/deploy-skills.sh`
   (`--status` audit, `--committed` for HEAD); the host `~/.agents` is a
   **deployment target** — direct edits there are dead ends; verify =
   `--status` clean (rsync reports; prune covers retirement). No cp
   loops, no name lists.
2. **`knot-init` step 4a** → "run `scripts/deploy-skills.sh`" (+ the
   same two-line host-target note); skill minor bump 4.14.0 → 4.15.0
   per this repo's skill-commit convention.
3. **K6 candidate (recorded, not committed to):** a check that no
   install-loop copy exists outside `scripts/deploy-skills.sh`, or that
   any documented skill list equals the script's glob.

## 4. Acceptance criteria

- [ ] `grep -c "personal skills repository" AGENTS.md` → 0
- [ ] no `cp -r .agents/skills` loop remains in `AGENTS.md` or
      `knot-init/SKILL.md`; both name `scripts/deploy-skills.sh`
- [ ] `knot-init` 4.15.0, version-bumped commit per convention
- [ ] `scripts/deploy-skills.sh --status` clean against the rewritten
      docs (docs and script agree — the K6 spirit, checked by hand once)
