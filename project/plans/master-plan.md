# Master Plan — Project Index

> **Last Updated:** 2026-08-23 (plan 072 drafted)

## How to Add a Plan

Each plan file must contain a title (e.g. `# Plan: Plan Name`).

To add it to this index:

1. Add a row to the Master Progress Table: number, link, status, date.
2. Optionally add an overview section below with **goal** only. The goal states **what** the plan covers — not why or how. Full details belong in the plan file.

**Ordering:** Plans are ordered by creation date, latest first, within the table. Unknown dates (`—`) appear last.

---

## Purging Old Completed Plans

When updating `master-plan.md`, **remove any plan that is `✅ Complete` and meets this criteria:**

1. **Completed more than 4 weeks ago** — use completion date from the plan file's Implementation Status

Rationale: Once a plan has been complete for a significant period, its status in the index no longer provides active value. The plan file itself (in `project/plans/`) remains as historical documentation. Only the index entry is removed.

**What to remove:**
- The row from the **Master Progress Table**
- Any overview section for this plan

**What to keep:**
- The plan file in `project/plans/` — historical documentation
- **Do NOT renumber** — leave gaps in numbering to preserve historical references

**What NOT to remove:**
- Plans marked `🟡 In Progress`, `⬜ Planned`, or `❌ Blocked` — regardless of age
- Plans that are `✅ Complete` but completed within the last 4 weeks
- Plans with active dependencies (other plans that reference this one)

**What IS removed (after 4 weeks):**
- Plans marked `✅ Complete` and older than 4 weeks
- Plans marked `⬜ Planned (superseded by ...)` and older than 4 weeks — rationale should be captured in a design document

---

## Master Progress Table

| # | Plan | Status | Created |
|---|------|--------|---------|
| 72 | [Clear Loom-Logs and Rig-Log at Startup](072-startup-log-clear/startup-log-clear-plan.md) | 🟡 In Progress | 2026-08-23 |
| 71 | [Record the Pi Session ID in Tie-Off Sections](071-tie-off-session-id/tie-off-session-id-plan.md) | 📝 Draft | 2026-08-22 |
| 70 | [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](070-dispatch-filename-collision/dispatch-filename-collision-plan.md) | ✅ Complete | 2026-08-22 |
| 69 | [Model Aliases — Rig-Level Model Registry](069-model-aliases/model-aliases-plan.md) | ✅ Complete | 2026-08-20 |
| 68 | [Rig Repository Separation](068-rig-repo-separation/rig-repo-separation-plan.md) | 📝 Draft | 2026-08-17 |
| 66 | [Relax the Knot Terminology Rule](066-terminology-rule-relaxation/terminology-rule-relaxation-plan.md) | 📝 Draft | 2026-07-29 |
| 63 | [Spurious Delete Suppression](063-spurious-delete-suppression/spurious-delete-suppression-plan.md) | 📝 Draft | 2026-07-16 |
| 31 | [Agent Profile Skills](agent-profile-skills.md) | ⬜ Planned | 2026-06-16 |
| 22 | [Notify Sender Leak Fix — Immediate Cascade Drain](notify-sender-leak-fix.md) | ⬜ Planned | 2026-06-11 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | 🟡 In Progress | 2026-06-08 |

---

_Overview sections for active and recently completed plans go here._

### 72. Clear Loom-Logs and Rig-Log at Startup

**Status:** 🟡 In Progress
**Created:** 2026-08-23
**Goal:** Truncate the rig-log and every loom-log at knot startup (after legacy migration, before discovery) so each run's logs contain only current-run events — removing unbounded cross-run growth and the repeated `WARN:` console spam from stale unparseable lines, while tie-offs remain the durable audit history.

Full details in [072-startup-log-clear/startup-log-clear-plan.md](072-startup-log-clear/startup-log-clear-plan.md).

### 71. Record the Pi Session ID in Tie-Off Sections

**Status:** 📝 Draft
**Created:** 2026-08-22
**Goal:** Record the pi session ID captured during agent execution as an optional `session:` metadata line in tie-off sections, so every tie-off section is traceable to the exact pi session that produced it.

Full details in [071-tie-off-session-id/tie-off-session-id-plan.md](071-tie-off-session-id/tie-off-session-id-plan.md).

### 70. Unique Event Dispatch Filenames — Per-Batch Sequence Suffix

**Status:** ✅ Complete (2026-08-22)
**Created:** 2026-08-22
**Goal:** Make event dispatch filenames unique per dispatch batch — same-second fan-out of one event type to one consumer gets per-directory sequence suffixes (`event-{ts}-001.md` …) and atomic `create_new` file creation, so no two dispatches can target one path.

Completed in Knot 0.33.0: pure `event_file_name(ts, seq)` helper; `seq: u32` on `EventDispatcherPort::dispatch` with use-case batch sequencing (singleton → plain name, N > 1 → `-001…-NNN` in emission order); atomic creation with bounded taken-name fallback; `EventsDispatched` loom-log entries carry the created file path (legacy 3-tuple lines skip with a warning); end-to-end acceptance test replaying the 2026-08-22 four-way `ValidationFail` incident. Design knowledge in [design/design-event-dispatch.md](../design/design-event-dispatch.md).

Full details in [070-dispatch-filename-collision/dispatch-filename-collision-plan.md](070-dispatch-filename-collision/dispatch-filename-collision-plan.md).

### 69. Model Aliases — Rig-Level Model Registry

**Status:** ✅ Complete (2026-08-20)
**Created:** 2026-08-20
**Goal:** Add a rig-level model registry (`rig/models.yml`) mapping aliases to `{provider, model}` pairs, plus an optional `model-ref` field on profiles (alias takes highest priority over direct spec), so swapping a model is a one-line edit picked up live on the next strand — no per-profile edits, no restart.

Completed in Knot 0.32.0: `ModelRegistry` domain value, `ModelRegistryPort` + `FileSystemModelRegistry` (fresh read per resolution — live swap), `model-ref` profile frontmatter with alias-over-direct precedence, state visibility (resolved provider/model, null when unresolvable), `run_startup` auto-creates `rig/models.yml`, and updated skills (knot-update/create/inspect/init).

Full details in [069-model-aliases/model-aliases-plan.md](069-model-aliases/model-aliases-plan.md).

### 63. Spurious Delete Suppression

**Status:** 📝 Draft
**Created:** 2026-07-16
**Goal:** Suppress spurious DELETE events caused by atomic file rewrites (truncate+write) using a configurable 5-second suppression window in the debounce engine.

Full details in [063-spurious-delete-suppression/spurious-delete-suppression-plan.md](063-spurious-delete-suppression/spurious-delete-suppression-plan.md).

### 31. Agent Profile Skills

**Status:** ⬜ Planned
**Created:** 2026-06-16
**Goal:** Add `skills` field to agent profiles so Knot passes `--no-skills` + `--skill <path>` to `pi`, making the agent's skill set explicit and keeping context concise.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [agent-profile-skills.md](agent-profile-skills.md).

### 22. Notify Sender Leak Fix — Immediate Cascade Drain

**Status:** ⬜ Planned
**Created:** 2026-06-11
**Goal:** Split `NotifyEventSource` senders from callback state so channels close immediately on drop, removing the 5-second timeout safety net.

Full details in [notify-sender-leak-fix.md](notify-sender-leak-fix.md).

### 20. Knot Modification Observability and Path Resolution Consistency

**Status:** 🟡 In Progress
**Created:** 2026-06-08
**Completed (Phase 0):** 2026-06-15
**Goal:** Make `KnotModified` filesystem changes observable via loom-log (`LoomEvent::KnotUpdated`), log parse failures to stderr, and ensure path resolution is consistent between initial load and file-watcher events.

**Result (Phase 0):** `NotifyEventSource` now receives correct `project_root` (parent of rig directory) so relative `strand_dir` paths resolve identically to `FileSystemLoomRepository::scan()`. Full rename `base_dir` → `rig_dir` across all 7 source files + 17 test files to eliminate ambiguity between "rig directory" and "project root". Remaining phases (KnotUpdated loom-log, parse failure logging, integration test) still pending.

Full details in [plan-knot-modify-observability.md](plan-knot-modify-observability.md).
