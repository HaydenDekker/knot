# Master Plan — Project Index

> **Last Updated:** 2026-06-10
> **Plan Added:** Static Output Paths and Log Timestamps

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
| 21 | [Static Output Paths and Log Timestamps](static-output-paths-and-timestamps.md) | ✅ Complete | 2026-06-10 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | ⬜ Planned | 2026-06-08 |
| 19 | [Fix KnotModified race and GET knot-status hang](plan-bugfix-knot-race-and-status-hang.md) | ✅ Complete | 2026-06-08 |
| 18 | [Sync Integration Tests to Async Layer](test-api-sync-async-layer.md) | ✅ Complete | 2026-06-08 |
| 17 | [lib.rs Composition Root and Inbound Adapter Tidy](lib-inbound-tidy.md) | ✅ Complete | 2026-06-08 |
| 16 | [Generic Task Management Tests](generic-task-management.md) | ✅ Complete | 2026-06-07 |
| 15 | [Integration Test Refactor](integration-test-refactor.md) | ✅ Complete | 2026-06-06 |
| 14 | [Loom/Knot Auto-Discovery and Knot CRUD API](loom-knot-auto-discovery-and-knot-crud.md) | ✅ Complete | 2026-06-07 |
| 13 | [Loom Naming Convention, Knot Definition Rules, and Discovery Fix](loom-knot-definition-and-discovery.md) | ✅ Complete | 2026-06-06 |
| 12 | [Tie-Off Append and Event Context](tie-off-append-and-event-context.md) | ✅ Complete | 2026-06-05 |
| 11 | [Loom Lifecycle Watching](loom-lifecycle-watching.md) | ✅ Complete | 2026-06-05 |
| 10 | [Knot-Per-Strand Config and Loom-Log State](knot-per-strand-and-loom-log-state.md) | ✅ Complete | 2026-06-04 |
| 9 | [Knot Skills and Swagger UI](knot-skills-and-swagger.md) | ✅ Complete | 2026-06-04 |
| 8 | [Rename Workspace → Rig](rename-workspace-to-rig.md) | ✅ Complete | 2026-06-04 |
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

### 21. Static Output Paths and Log Timestamps

**Status:** ✅ Complete
**Created:** 2026-06-10
**Completed:** 2026-06-11
**Goal:** Make tie-off output paths and loom-log paths static (derived from loom/knot names under `rig/output/`), remove `tie-off-dir` from knot YAML, and add ISO 8601 timestamps to console logs and loom-log events.

**Result:** `tie_off_dir` removed from domain and KnotFile. Paths statically derived: `rig/output/{loom-id}/{knot-name}/{strand}.output` and `rig/output/{loom-id}/.loom-log`. ISO 8601 timestamps on all console logs and LoomEvent variants. 278 tests pass (196 lib + 82 integration, 1 ignored).

Full details in [static-output-paths-and-timestamps.md](static-output-paths-and-timestamps.md).

### 20. Knot Modification Observability and Path Resolution Consistency

**Status:** ⬜ Planned
**Created:** 2026-06-08
**Goal:** Make `KnotModified` filesystem changes observable via loom-log (`LoomEvent::KnotUpdated`), log parse failures to stderr, and ensure path resolution is consistent between initial load and file-watcher events.

Full details in [plan-knot-modify-observability.md](plan-knot-modify-observability.md).

### 19. Fix KnotModified race and GET knot-status hang

**Status:** ✅ Complete
**Created:** 2026-06-08
**Completed:** 2026-06-08
**Goal:** Fix `KnotModified` recovery when `LoomAdded` fires before knot file is fully written (loom registered with 0 knots), and wrap `GET /looms/{id}/knots/{name}` in `spawn_blocking` to prevent blocking the axum worker thread.

**Result:** `handle_knot_modified` now recovers by registering missing knots. `get_knot_status` uses `tokio::task::spawn_blocking`. 5 new tests (3 unit, 2 integration), all passing.

Full details in [plan-bugfix-knot-race-and-status-hang.md](plan-bugfix-knot-race-and-status-hang.md).

### 18. Sync Integration Tests to Async Layer

**Status:** ✅ Complete
**Created:** 2026-06-08
**Completed:** 2026-06-08
**Goal:** Fix 8 test files that use stale spawn_server/wait_for_port/HTTP helper signatures, bringing them up to the async layer API defined in ADR-002/003.

**Result:** 241 tests pass (0 failed, 1 ignored), full suite in 11s.

Full details in [test-api-sync-async-layer.md](test-api-sync-async-layer.md).

### 17. lib.rs Composition Root and Inbound Adapter Tidy

**Status:** ✅ Complete
**Created:** 2026-06-08
**Completed:** 2026-06-08
**Goal:** Remove dead `graceful_shutdown` from `lib.rs`, extract composition root into `src/server.rs`, split `inbound/mod.rs` (2211 lines) into `types.rs` + `loom.rs` + `system.rs` + `router.rs`, and move `health`/`list_agents` handlers into `inbound/system.rs`.

**Result:** `lib.rs` reduced from 440→18 lines, `inbound/mod.rs` from 2211→18 lines, all 224 tests pass.

Full details in [lib-inbound-tidy.md](lib-inbound-tidy.md).

### 16. Generic Task Management Tests

**Status:** ✅ Complete
**Created:** 2026-06-07
**Completed:** 2026-06-07
**Goal:** Create `tests/generic_task_management.rs` — 10 tokio-only tests validating the channel-cascade shutdown pattern (JoinSet ownership, cooperative drain, abort safety net) with zero Knot domain types.

Full details in [generic-task-management.md](generic-task-management.md).

### 15. Integration Test Refactor

**Status:** ✅ Complete
**Created:** 2026-06-06
**Completed:** 2026-06-06
**Goal:** Split 3272-line `tests/integration.rs` into 10 feature-focused modules with shared infrastructure, reducing ~31 tests to ~25 through consolidation of duplicate pipeline variants.

Full details in [integration-test-refactor.md](integration-test-refactor.md).

### 14. Loom/Knot Auto-Discovery and Knot CRUD API

**Status:** ✅ Complete
**Created:** 2026-06-07
**Completed:** 2026-06-08
**Goal:** Watch the rig and loom directories for filesystem events so new looms, new knots, edited knots, and deleted knots are active in real time without restart. Add HTTP CRUD endpoints for individual knots. Remove `POST /looms/discover`.

**Result:** `ConfigEvent` type and `ConfigEventHandler` use case process filesystem changes. `NotifyEventSource` watches rig and loom directories. `ManageKnot` use case and 3 new HTTP endpoints (POST/PATCH/DELETE `/looms/{id}/knots/{name}`). `POST /looms/discover` removed. 9 new integration tests in `tests/auto_discovery_and_knot_crud.rs`. 191/192 tests pass (1 pre-existing subprocess flake).

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [loom-knot-auto-discovery-and-knot-crud.md](loom-knot-auto-discovery-and-knot-crud.md).

### 13. Loom Naming Convention, Knot Definition Rules, and Discovery Fix

**Status:** ✅ Complete
**Created:** 2026-06-06
**Completed:** 2026-06-06
**Goal:** Fix loom discovery to use `-loom` suffix filter, make `strand_dir` and `tie_off_dir` required per-knot fields, remove ambiguous `Loom.source_dir`, and rewrite `POST /looms` to create loom directories with knot files.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [loom-knot-definition-and-discovery.md](loom-knot-definition-and-discovery.md).

### 12. Tie-Off Append and Event Context

**Status:** ✅ Complete
**Created:** 2026-06-05
**Completed:** 2026-06-05
**Goal:** Tie-off files append new agent responses as `---`-separated sections with event metadata headers. Delete events trigger the agent with context about the deletion. The agent receives event type and previous tie-off content.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 11. Loom Lifecycle Watching

**Status:** ✅ Complete
**Created:** 2026-06-05
**Completed:** 2026-06-05
**Goal:** Wire `EventSource` into `RegisterLoom`, `UnregisterLoom`, and implement `POST /looms/discover` so looms can be added, discovered, and removed at runtime without restart.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

### 10. Knot-Per-Strand Config and Loom-Log State

**Status:** ✅ Complete
**Created:** 2026-06-04
**Completed:** 2026-06-04
**Goal:** Move source/tie-off config into each knot (removing loom-level `.loom-config.yaml`), and consolidate knot-state events into the loom-log.

### 9. Knot Skills and Swagger UI

**Status:** ✅ Complete
**Created:** 2026-06-04
**Completed:** 2026-06-04
**Goal:** Add utoipa-generated Swagger UI to Knot, create three AI skills (knot-init, knots-and-looms, knot-inspect) and verify them with integration tests.

**PRD:** [Knot Skills — AI-Driven Configuration via Skills](../prds/prd-knot-skills.md)

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
