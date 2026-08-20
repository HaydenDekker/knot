# Master Plan — Project Index

> **Last Updated:** 2026-08-20 (plan 069 added)

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
| 69 | [Model Aliases — Rig-Level Model Registry](069-model-aliases/model-aliases-plan.md) | 📝 Draft | 2026-08-20 |
| 68 | [Rig Repository Separation](068-rig-repo-separation/rig-repo-separation-plan.md) | 📝 Draft | 2026-08-17 |
| 66 | [Relax the Knot Terminology Rule](066-terminology-rule-relaxation/terminology-rule-relaxation-plan.md) | 📝 Draft | 2026-07-29 |
| 63 | [Spurious Delete Suppression](063-spurious-delete-suppression/spurious-delete-suppression-plan.md) | 📝 Draft | 2026-07-16 |
| 31 | [Agent Profile Skills](agent-profile-skills.md) | ⬜ Planned | 2026-06-16 |
| 22 | [Notify Sender Leak Fix — Immediate Cascade Drain](notify-sender-leak-fix.md) | ⬜ Planned | 2026-06-11 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | 🟡 In Progress | 2026-06-08 |

---

_Overview sections for active and recently completed plans go here._

### 69. Model Aliases — Rig-Level Model Registry

**Status:** 📝 Draft
**Created:** 2026-08-20
**Goal:** Add a rig-level model registry (`rig/models.yml`) mapping aliases to `{provider, model}` pairs, plus an optional `model-ref` field on profiles (alias takes highest priority over direct spec), so swapping a model is a one-line edit picked up live on the next strand — no per-profile edits, no restart.

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
