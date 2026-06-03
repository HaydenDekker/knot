# Master Plan — Project Index

> **Last Updated:** 2026-06-03

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
- Any overview section for that plan

**What to keep:**
- The plan file in `project/plans/` — historical documentation
- **Do not renumber** — leave gaps in numbering to preserve historical references

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
| 5 | [Loom HTTP Interface](loom-http-interface.md) | ⬜ Planned | 2026-06-03 |
| 4 | [Agent Execution and Tie-off Generation](agent-execution.md) | ⬜ Planned | 2026-06-03 |
| 3 | [File Watcher with Debounce](file-watcher.md) | ⬜ Planned | 2026-06-03 |
| 2 | [Loom Discovery and State Files](loom-discovery-and-state.md) | ⬜ Planned | 2026-06-03 |
| 1 | [Knot Domain Models](knot-domain-models.md) | ⬜ Planned | 2026-06-03 |

---

## Plan Overviews

_Overview sections for active and recently completed plans go here._

### 1. Knot Domain Models

**Status:** ⬜ Planned
**Created:** 2026-06-03
**Goal:** Define core domain types and knot file parsing — the foundation all other plans build on.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 2. Loom Discovery and State Files

**Status:** ⬜ Planned
**Created:** 2026-06-03
**Goal:** Discover looms from the filesystem and maintain loom-log / knot-state files.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 3. File Watcher with Debounce

**Status:** ⬜ Planned
**Created:** 2026-06-03
**Goal:** Watch source directories for strand events with per-file 100ms debouncing.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 4. Agent Execution and Tie-off Generation

**Status:** ⬜ Planned
**Created:** 2026-06-03
**Goal:** Process strand events by invoking the agent CLI and writing tie-offs.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 5. Loom HTTP Interface

**Status:** ⬜ Planned
**Created:** 2026-06-03
**Goal:** HTTP endpoints for loom/knot observability and management, sourced from filesystem state.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)
