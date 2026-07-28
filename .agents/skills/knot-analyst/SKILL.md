---
name: knot-analyst
description: "Analyse rig productivity and project progress at runtime. Tail the rig-log and loom-logs, inspect git history, assess project completion against plan documents, and identify blockers. USE FOR: analyse rig, rig analysis, rig productivity, project progress, how far along, rig health, rig performance, blocker detection, stalled rig, rig diagnosis, activity analysis, progress report, rig review, project health check, rig assessment, knot productivity. DO NOT USE FOR: creating looms (use knot-create), modifying looms (use knot-create), initialising a rig (use knot-init), inspecting raw state (use knot-inspect), designing knots (use knot-design)."
license: MIT
metadata:
  author: Knot Team
  version: "1.1.0"
  compatibility: "Knot 0.26.0+"
---

# Knot Analyst Skill

Analyse the productivity and progress of a running Knot rig. This skill
combines operational signals (rig-log, loom-logs, git history, knot
processing state) with project-document signals (plans, specs, decision
records) to produce a structured assessment of how the rig is performing
and whether the project is making progress.

**Rig-log:** `rig/.rig-log` (append-only JSONL — operational events)
**Loom-logs:** `rig/tie-offs/{loom-id}/.loom-log` (append-only JSONL —
per-loom activity)
**State file:** `rig/state.json` (current rig snapshot)

---

## Core Philosophy

### Holistic View

Productivity is not just "are knots running." It is: are the right
things happening, at a reasonable pace, without blockers or wasted
cycles? The analyst combines operational data with project-document
signals to answer this.

### File-First

All analysis reads from files — no HTTP calls needed. Read `rig/.rig-log`,
loom-logs, `rig/state.json`, git log, and project documents.

### Signal-Based, Not Prescriptive

The analyst identifies patterns and flags concerns. It does not fix
things or modify configuration. Use `knot-create` or `knot-inspect`
for action.

### Generic Terminology

The analyst works with **any project** that uses Knot. It does not
assume a specific document structure. It looks for the project's own
"definition of done" artifacts — whatever the project has chosen to
track progress. The skill uses generic terms: plan documents,
acceptance specifications, completion criteria.

---

## Prerequisites

1. Knot must be running and `rig/state.json` must exist.
   If the file does not exist, report: "Knot is not running or rig is
   not initialised. Use `knot-init` skill."

---

## Agent Workflow

### Full Rig Analysis

When asked to analyse the rig, assess progress, or give a productivity
report, run through all analysis dimensions below and present a
structured report.

---

### Dimension 1 — Operational Activity

Determine whether the rig has been doing meaningful work.

**Read `rig/.rig-log`:**

The rig-log is an append-only JSONL file recording serious operational
events. Tail the last 50 lines (or the full file if smaller).

Look for:

| Signal | What to extract | Interpretation |
|--------|-----------------|----------------|
| `TimeoutExceeded` events | Count, which knots, frequency | Repeated timeouts indicate the agent is struggling with its workload — too complex a task, insufficient timeout, or a stuck session |
| `QueueIdle` events | Timestamps between idle periods | Long idle gaps mean the rig is waiting for input. Frequent idle means work is completing quickly. A single idle at the end with no follow-up means work has stopped |
| Age of last entry | Compare to current time | If the last entry is hours or days old, the rig may have stalled or completed all work |

**Read each loom-log** at `rig/tie-offs/{loom-id}/.loom-log`:

Tail the last 30 lines per loom. Look for:

| Signal | Interpretation |
|--------|---------------|
| `KnotFailed` events | Knot encountered errors. Note which strand and error message |
| `KnotCompleted` events | Successful processing. Compare count to failures |
| `KnotProcessing` without `KnotCompleted` | Knot started but never finished — likely timed out or crashed |
| `KnotParseWarning` events | Knot definition has issues (unknown frontmatter fields) |
| Repeated `KnotCompleted` on the same strand | The knot is re-triggering. If the tie-off says "no changes needed" each time, the strand is stale (see Dimension 4) |
| `SessionResumed` events | Session retries occurred. High retry counts signal fragile invocations |
| `StrandSkipped` with reason `"filtered temp file"` | Expected filesystem noise — a temp file from `sed -i` or similar tool triggered an event but was filtered before processing. These are informational only and do not indicate a problem. Count them to gauge noise levels but do not flag as issues. |
| `StrandSkipped` with reason `"missing file (unknown pattern)"` | A file triggered a filesystem event but was deleted before processing. The event watcher fires instantly, but the file may be short-lived (a script creates, reads, and deletes it within milliseconds). The event is persisted in `rig/events/*.json` and auto-removed on pop — it does not recur from the same event. If frequent for the same path, investigate what is creating/deleting files in the strand directory. |

**Produce a summary:**

```
## Operational Activity

| Loom | Completed | Failed | Timeouts | Retries | Last Activity |
|------|-----------|--------|----------|---------|---------------|
| `planning-loom` | 12 | 1 | 0 | 2 | 5 min ago |

Rig-log highlights: 3 timeouts on `coder` knot in last 24h.
```

---

### Dimension 2 — Git History

Git history provides an independent measure of whether work is
producing tangible output.

**Run:** `git log --oneline --since="7 days ago" | head -30`

Analyze:

| Signal | Interpretation |
|--------|---------------|
| Commit count (last 7 days) | High count = active. Zero commits = no output reaching git (knots may be running but not committing, or `git-versioned: false`) |
| Commit authors | If all commits are from the same agent/profile, the rig is active. If mixed with human commits, collaboration is happening |
| Commit messages | Look for Knot-generated messages (tie-off commits). Absence of Knot commits means either `git-versioned: false` or no successful knot runs |
| Files touched | Are the expected output directories being modified? (e.g. `project/plans/`, `project/adrs/`, `src/`) |
| Time since last commit | More than 24h with no commits and active knots = something is producing output but not persisting it, or the rig is idle |

**Run:** `git diff --stat HEAD~5` (if enough commits exist)

This shows the volume of changes over the last 5 commits — gives a
sense of whether the rig is making meaningful edits or only touching
configuration files.

**Produce a summary:**

```
## Git History (last 7 days)

Commits: 24
Last commit: 2 hours ago
Files most modified: project/plans/ (14), project/adrs/ (6), src/ (4)
Trend: Active — consistent commit frequency
```

---

### Dimension 3 — Project Document Progress

The project should define its own completion criteria in documents.
The analyst inspects whatever progress-tracking documents exist.

**Look for plan documents** in `project/plans/`:

If a `project/plans/` directory exists, check for a master plan index
(e.g. `master-plan.md` or similar). If found, read it and extract:

- List of plans and their status (Draft, Active, Complete)
- Ratio of completed to total plans
- Which plans are active and how long they have been active

**Look for phase documents** inside plan directories
(e.g. `project/plans/001-slug/`):

If phase documents exist (files matching `*-phase-*.md`), check for:
- Phase task completion indicators (checkmarks, done markers, or
  completion notes within the phase document)
- How many phases have notes about being completed
- Any phases with "blocker" or "stuck" language in their notes

**Look for acceptance specifications** in `project/acceptance/`:

If a `project/acceptance/` directory exists, check:
- How many specs exist vs. how many plans or PRDs exist
- Whether specs have environment tags or validation references
- If a master BDD index exists, whether it shows gaps (PRDs without
  matching specs)

**Look for decision records** in `project/adrs/`:

If a `project/adrs/` directory exists, check:
- Count of ADRs (gauge of architectural progress)
- Any ADRs with "Draft" or "Superseded" status
- Recent ADR activity vs. plan activity

**Produce a summary:**

```
## Project Progress

Plans: 3 total — 1 Complete, 1 Active (14 days), 1 Draft
Phases: 8 total — 5 complete, 3 in progress
Acceptance specs: 2 of 3 plans have matching specs
Decision records: 7 ADRs, all approved

Overall: ~60% complete. Active plan has been running 14 days —
check for blockers.
```

> **Note:** The analyst does not assume any specific document format.
> It reads whatever documents exist and reports what it finds. If no
> progress-tracking documents are found, it reports: "No progress
> tracking documents detected in `project/`. Cannot assess project
> completion."

---

### Dimension 4 — Stagnation Detection

Identify whether the rig appears stuck, oscillating, or producing no
forward progress.

**Check knot status from `rig/state.json`:**

| Pattern | Concern Level | Meaning |
|---------|--------------|---------|
| All knots `idle` for > 1 hour | Medium | Rig is configured but not receiving triggers. May be waiting for input strands or events |
| All knots `idle` with no recent git activity | High | Rig may be dead — no work happening anywhere |
| Knot stuck in `processing` | High | Agent session may be hung or the provider is unresponsive |
| Mix of `completed` and `idle` | Normal | Healthy state after work completes |

**Check for repeated processing without changes:**

Read the tie-off files (`rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`)
for the last 5 entries. If multiple consecutive entries say "no changes
needed" or similar language for the **same strand**, the strand may be
stale — it is triggering the knot but the knot has already done its
work.

| Pattern | Meaning |
|---------|---------|
| Same strand, 3+ "no changes" entries | Strand is stale. The input file has not changed meaningfully but is triggering re-processing (possibly due to metadata-only edits) |
| Same strand, alternating "changes applied" / "no changes" | Loop oscillation. Two knots may be fighting over the same file |
| Different strands, all "no changes" | The rig has converged on its current inputs — all work is done for the current set of strands |

**Check for error accumulation:**

From loom-logs, count `KnotFailed` events per knot in the last 24 hours.
If a single knot has 3+ failures, flag it as a recurring problem.

**Produce a summary:**

```
## Stagnation Check

Knot status: 4 idle, 1 completed, 0 processing
Stale strands: `planning-loom/prd-planner` — same strand re-processed 5 times with "no changes"
Recurring failures: `coder` knot — 4 failures in last 24h (timeout on large files)
Loop oscillation: None detected

Assessment: Rig is idle. No active work. Stale strand in planning-loom
may indicate the PRD has not been updated.
```

---

### Dimension 5 — Blocker Identification

Synthesise findings across all dimensions to identify specific blockers.

**Known blocker patterns:**

| Pattern | Dimensions involved | Action to suggest |
|---------|-------------------|-------------------|
| **Timeout wall** | Operational (rig-log timeouts) + Git (no recent commits from affected knot) | Increase profile timeout, or reduce task complexity in knot instructions |
| **Stale input** | Stagnation (same strand repeated) + Git (no new commits to strand directory) | Input directory has not been updated — the rig is waiting for new work |
| **Loop oscillation** | Stagnation (alternating tie-offs) + Operational (high processing count) | Check knot authority boundaries. One knot may be overwriting the other's output |
| **Missing profile** | Operational (`ProfileNotFound` errors) + State (knot references missing profile) | Create the referenced profile in `rig/profiles/` |
| **Parse warnings** | Operational (`KnotParseWarning` events) | Fix knot frontmatter — unknown YAML fields will cause silent degradation |
| **Event dead-letter** | Operational (no consumer processing) + State (event dispatch dirs with files but consumer idle) | Consumer knot may have wrong `strand-dir` event URI. Verify subscription matches producer |
| **Plan stall** | Project docs (active plan > 7 days with no phase progress) + Git (low commit count) | The plan may have blockers. Check phase documents for "stuck" notes |
| **Provider fatigue** | Operational (increasing timeout rate over time) + Git (declining commit quality/count) | Provider rate limits or model degradation. Check provider status or switch profile |

**Produce a structured blocker list:**

```
## Blockers

1. **Timeout wall** — `coder` knot timing out on files in `src/core/`.
   4 failures in 24h. Profile timeout is 300s.
   → Suggestion: Increase timeout to 600s or split the task across
     multiple knots with smaller strand directories.

2. **Stale input** — `prd-planner` re-processing the same PRD 5 times
   with "no changes needed". PRD last modified 3 days ago.
   → Suggestion: No action needed — the rig has converged on this PRD.
     New PRD input will re-trigger work.

3. **Plan stall** — Plan 002 has been active for 14 days. Phase 3
   has no completion notes.
   → Suggestion: Review phase 3 document for blockers or missing steps.
```

---

### Dimension 6 — Productivity Score

Combine all dimensions into a simple traffic-light assessment.

| Score | Criteria |
|-------|----------|
| 🟢 **Healthy** | Active commits in last 24h, no recurring failures, project documents show forward progress, no blockers detected |
| 🟡 **Caution** | Some timeouts or idle periods, but work is progressing. Minor blockers that do not stop forward movement |
| 🔴 **Blocked** | No commits in 48h + active knots, or recurring failures on core knots, or plan stalled > 7 days with no phase progress |
| ⚪ **Idle** | Rig configured but no work to do (all strands processed, no new input). This is not a problem — it means the rig is waiting |

---

## Full Analysis Report Template

When producing the full analysis report, use this structure:

```markdown
# Rig Analysis Report — {date}

## Productivity Score: 🟢 / 🟡 / 🔴 / ⚪

## Executive Summary
2-3 sentences on overall rig health and project trajectory.

## Operational Activity
| Loom | Completed | Failed | Timeouts | Retries | Last Activity |
|------|-----------|--------|----------|---------|---------------|

Rig-log highlights: ...

## Git History (last 7 days)
- Commits: N
- Last commit: X ago
- Files most modified: ...
- Trend: ...

## Project Progress
Plans / Phases / Acceptance specs / Decision records summary.
Overall completion estimate.

## Stagnation Check
Knot status summary. Stale strands. Recurring failures. Loop detection.

## Blockers
Numbered list of identified blockers with suggested actions.

## Recommendations
2-4 concrete actions to improve productivity or address blockers.
```

---

## Quick Checks

For lightweight, targeted queries:

### "Is the rig working?"

1. Read `rig/.rig-log` — last 10 lines. Any entries in the last hour?
2. Run `git log --oneline -5` — any commits in the last 24h?
3. Read `rig/state.json` — are any knots in `processing` or `completed`?

### "Where is the project?"

1. Look for `project/plans/` — read any master plan index file.
2. Count completed vs. active vs. draft plans.
3. Check phase documents for completion markers.
4. Report rough percentage and the active plan's status.

### "Is anything broken?"

1. Tail `rig/.rig-log` for `TimeoutExceeded` events in last 24h.
2. Tail each loom-log for `KnotFailed` events in last 24h.
3. Check `rig/state.json` for knots with `last_error` set.
4. Report any non-zero findings.

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `rig/.rig-log` does not exist | Rig-log may not have been written yet (no serious events). Report "No rig-log found — no timeout or idle events recorded." |
| `rig/state.json` does not exist | Knot is not running. Report and suggest `knot-init`. |
| Git is not initialised | Project may not be in a git repo. Skip git analysis, note in report. |
| No `project/` directory | Cannot assess project document progress. Report "No project documents found." |
| Loom-log missing for a loom | Loom has no recorded activity. Report "No activity for `{loom-id}`." |
| Tie-off file is empty or missing | Knot has not yet produced output. Not an error. |

---

## Quick Reference

```bash
# Tail rig-log for recent issues
tail -50 rig/.rig-log

# Count timeouts in last 24h
grep "TimeoutExceeded" rig/.rig-log | tail -20

# Check loom activity
for log in rig/tie-offs/*/.loom-log; do echo "=== $log ==="; tail -5 "$log"; done

# Recent git activity
git log --oneline --since="7 days ago" | head -20

# Files touched recently
git diff --stat HEAD~5

# Knot statuses
python3 -m json.tool rig/state.json | grep -A2 '"status"'
```

---

## Cross-Reference

**Before using this skill:** Read the `knot-abstractions` skill for the
layered architecture overview. Understanding the rig/profile/skill/application
boundary is essential for correctly interpreting rig productivity signals.

Related skills:

1. **knot-abstractions skill** — foundational architecture overview
2. **knot-inspect skill** — read raw rig state and loom activity
3. **knot-create skill** — fix configuration issues found by analysis
4. **knot-design skill** — redesign knots if oscillation or authority
   issues are detected

This skill (`knot-analyst`) provides the **interpretive layer** over raw
state. Use knot-inspect for data, knot-create for fixes.
