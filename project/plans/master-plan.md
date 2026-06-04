# Master Plan — Project Index

> **Last Updated:** 2026-06-04 (plan 7 completed)

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
| 8 | [Rename Workspace → Rig](rename-workspace-to-rig.md) | ⬜ Planned | 2026-06-04 |
| 7 | [pi Agent Integration](pi-agent-integration.md) | ✅ Complete | 2026-06-04 |
| 6 | [Loom Config, Path Resolution and Agent Error Logging](loom-config-and-path-resolve.md) | ✅ Complete | 2026-06-04 |
| 5 | [System Integration and Wiring](system-integration.md) | ✅ Complete | 2026-06-03 |
| 4 | [Loom HTTP Interface](loom-http-interface-handler.md) | ✅ Complete | 2026-06-03 |
| 3 | [Outbound Adapters](file-watcher.md) | ✅ Complete | 2026-06-03 |
| 2 | [Application Layer — Ports and Use Cases](loom-discovery-and-state.md) | ✅ Complete | 2026-06-03 |
| 1 | [Knot Domain Models](knot-domain-models.md) | ✅ Complete | 2026-06-03 |

---

## Plan Overviews

_Overview sections for active and recently completed plans go here._

### 1. Knot Domain Models

**Status:** ✅ Complete
**Created:** 2026-06-03
**Completed:** 2026-06-03
**Hex Layer:** Domain
**Goal:** Domain entities, value objects, domain events, knot file format validation — zero IO, zero framework.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 2. Application Layer — Ports and Use Cases

**Status:** ✅ Complete
**Created:** 2026-06-03
**Completed:** 2026-06-03
**Hex Layer:** Application
**Goal:** Port traits, use cases, debounce engine, processing state machine — all tests use mock ports.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 3. Outbound Adapters

**Status:** ✅ Complete
**Created:** 2026-06-03
**Completed:** 2026-06-03
**Hex Layer:** Outbound Adapters
**Goal:** Concrete adapters for filesystem IO, notify watching, subprocess execution — all tests use `tempfile`.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 4. Loom HTTP Interface

**Status:** ✅ Complete
**Created:** 2026-06-03
**Completed:** 2026-06-03
**Hex Layer:** Inbound Adapter
**Goal:** Axum handlers and routes that call use cases — never touch adapters directly.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 5. System Integration and Wiring

**Status:** ✅ Complete
**Created:** 2026-06-03
**Completed:** 2026-06-04
**Hex Layer:** Composition Root
**Goal:** Bootstrap all layers, wire event pipeline, full end-to-end integration tests.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 6. Loom Config, Path Resolution and Agent Error Logging

**Status:** ✅ Complete
**Created:** 2026-06-04
**Completed:** 2026-06-04
**Hex Layer:** Outbound Adapters + Application
**Goal:** Canonical path resolution, `.loom-config.yaml` for external source/tie-off directories, and agent error logging in knot-state and loom-log.

### 7. pi Agent Integration

**Status:** ✅ Complete
**Created:** 2026-06-04
**Completed:** 2026-06-04
**Hex Layer:** Domain → Application → Outbound Adapters
**Goal:** Extend AgentConfig with provider/model/tools, construct `pi` CLI invocation from knot config, and pass strand content to the agent.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)
