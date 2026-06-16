# Master Plan — Project Index

> **Last Updated:** 2026-06-17

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
| 35 | [Rig Switching and Sharing](rig-switching-and-sharing.md) | ⬜ Planned | 2026-06-17 |
| 34 | [Strand Directory Auto-Creation](strand-dir-auto-create.md) | ✅ Complete | 2026-06-17 |
| 33 | [Queue Event Dedup — Prevent Duplicate Strand Processing](queue-event-dedup.md) | ✅ Complete | 2026-06-16 |
| 32 | [Simplify Agent Invocation — Remove --system-prompt](simplify-agent-invocation.md) | ✅ Complete | 2026-06-16 |
| 31 | [Agent Profile Skills](agent-profile-skills.md) | ⬜ Planned | 2026-06-16 |
| 30 | [Context Management — Slim Agent Prompt and Tie-Off Headers](context-management.md) | ✅ Complete | 2026-06-15 |
| 29 | [Auto-Discovery Reliability Fixes](auto-discovery-reliability.md) | ✅ Complete | 2026-06-15 |
| 28 | [Rig-Log Notification and Timeout Handling](rig-log-notification-and-timeout.md) | ✅ Complete | 2026-06-14 |
| 27 | [Git Versioning — Automatic Commit History for Agent Work](git-versioning.md) | ✅ Complete | 2026-06-13 |
| 26 | [HTTP Observability Only — Remove Control Endpoints](http-observability-only.md) | ✅ Complete | 2026-06-13 |
| 24 | [Tie-Off Output Rename and Knot File Cleanup](tieoff-output-rename-and-knot-cleanup.md) | ✅ Complete | 2026-06-12 |
| 23 | [Shared Agent Profiles](shared-agent-profiles.md) | ✅ Complete | 2026-06-11 |
| 22 | [Notify Sender Leak Fix — Immediate Cascade Drain](notify-sender-leak-fix.md) | ⬜ Planned | 2026-06-11 |
| 21 | [Static Output Paths and Log Timestamps](static-output-paths-and-timestamps.md) | ✅ Complete | 2026-06-10 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | ✅ Complete | 2026-06-08 |
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
| 3 | [Outbound Adapters](file-watcher.md) | ✅ Complete | 2026-06-03 (bugfix 2026-06-14) |
| 2 | [Application Layer — Ports and Use Cases](loom-discovery-and-state.md) | ✅ Complete | 2026-06-03 |
| 1 | [Knot Domain Models](knot-domain-models.md) | ✅ Complete | 2026-06-03 |

---

## Plan Overviews

_Overview sections for active and recently completed plans go here._

### 35. Rig Switching and Sharing

**Status:** ⬜ Planned
**Created:** 2026-06-17
**Goal:** Enable switching between multiple rigs on the same project and packaging rigs for sharing with colleagues by distributing loom definitions (excluding derived state).

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [rig-switching-and-sharing.md](rig-switching-and-sharing.md).

### 34. Strand Directory Auto-Creation

**Status:** ✅ Complete
**Created:** 2026-06-17
**Completed:** 2026-06-17
**Goal:** Automatically create a knot's `strand_dir` at registration time if it does not exist, logging the creation in the loom-log.

**Result:** `LoomEvent::DirectoryCreated` variant added to domain. `ConfigEventHandler` gained `ensure_strand_dir_and_watch` helper that creates missing `strand_dir` with `fs::create_dir_all` and logs the creation before registering the watcher. Covers initial registration, dynamic knot addition, and knot modification when `strand_dir` changes. 320 tests pass. Version bumped to 0.10.0.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [strand-dir-auto-create.md](strand-dir-auto-create.md).

### 33. Queue Event Dedup — Prevent Duplicate Strand Processing

**Status:** ✅ Complete
**Created:** 2026-06-16
**Completed:** 2026-06-16
**Goal:** Replace the debounce engine's output mpsc channel with an inspectable queue so duplicate events for the same strand are collapsed before reaching ProcessStrand.

**Result:** `InspectQueue<StrandEvent>` type with `push_or_replace` dedup by `(strand_path, loom_id, knot_id, event_type)` key. DebounceEngine emits into the queue instead of an opaque mpsc channel. ProcessStrand reads from the queue with notifier-based wait. Shutdown via `Option<StrandEvent>` sentinel. Different event types always pass through — only repeated events of the same type collapse. 316 unit + integration tests pass.

Full details in [queue-event-dedup.md](queue-event-dedup.md).

### 32. Simplify Agent Invocation — Remove --system-prompt

**Status:** ✅ Complete
**Created:** 2026-06-16
**Completed:** 2026-06-16
**Goal:** Remove `--system-prompt` CLI flag from `pi` invocation, rename `AgentProfile.system_prompt` → `profile_prompt`, and merge profile prompt + knot instructions + trigger line into a single stdin prompt. Eliminates knot instruction duplication and makes the profile prompt visible in session files.

**Result:** `AgentConfig::build_cli_args()` no longer accepts system prompt — simplified to `build_cli_args(&self)`. `ExecutionContext` gained `profile_prompt` field. `SubprocessAgentRunner::build_prompt_with_context()` builds prompt chain: profile prompt → knot instructions → trigger line. `resolve_agent_config()` return type simplified from 3-tuple to 2-tuple. Domain glossary updated. ADR-007 documents the decision. 21 files changed, 303+ tests pass. Version bumped to 0.8.0.

Full details in [simplify-agent-invocation.md](simplify-agent-invocation.md).

### 31. Agent Profile Skills

**Status:** ⬜ Planned
**Created:** 2026-06-16
**Goal:** Add `skills` field to agent profiles so Knot passes `--no-skills` + `--skill <path>` to `pi`, making the agent's skill set explicit and keeping context concise.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [agent-profile-skills.md](agent-profile-skills.md).

### 30. Context Management — Slim Agent Prompt and Tie-Off Headers

**Status:** ✅ Complete
**Created:** 2026-06-15
**Completed:** 2026-06-15
**Goal:** Remove full tie-off history from agent prompt (replaced with single trigger line), update tie-off section headers to single-line format, and remove `previous_tie_off` from `ExecutionContext`.

**Result:** Agent prompt now contains only: system prompt, knot instruction, input file via `@{path}`, and a short trigger line (`**knot-name** triggered by **event-type** on **file-name**`). Tie-off headers changed from three-line format to single-line (`## knot-name triggered by event-type file-name`). `previous_tie_off` field removed from `ExecutionContext`; `knot_name` added. 7 source files changed, 359 tests pass.

Full details in [context-management.md](context-management.md).

### 29. Auto-Discovery Reliability Fixes

**Status:** ✅ Complete
**Created:** 2026-06-15
**Completed:** 2026-06-15
**Goal:** Fix four reliability defects in the auto-discovery feature (Plan #14): path canonicalisation mismatch in rig watch, wasteful full rig re-scan on `LoomAdded`, missing loom path in `LoomAdded` events, and silent event drops when config channel is full.

**Result:** `ConfigEvent::LoomAdded` carries `loom_dir: String` for targeted scanning. `register_watch()` canonicalises rig paths via `resolve_path()` so notify absolute paths match. `handle_loom_added()` scans only the new loom directory via `LoomRepository::scan_knot_files()`. `ReloadConfig` use case + `POST /config/reload` endpoint provides manual recovery. 12 new tests across domain, outbound, application, inbound, and integration layers. Version bumped to 0.6.0. 303+ tests pass.

**PRD:** [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md)

Full details in [auto-discovery-reliability.md](auto-discovery-reliability.md).

### 28. Rig-Log Notification, Timeout Handling and Rollback

**Status:** ✅ Complete
**Created:** 2026-06-14
**Completed:** 2026-06-14
**Goal:** Rig-level event log (`rig/.rig-log`) records timeout and queue-idle events. On timeout, tie-off is preserved unchanged (error written to loom-log + rig-log only).

**Result:** `RigLogPath` and `RigLogEvent` domain types. `RigLogPort` trait + `FileSystemRigLog` adapter. `AgentProfile.timeout` field (optional, seconds) — parsed from profile frontmatter. `ExecutionContext.timeout` — per-context override with runner default fallback. `ProcessStrand` writes `TimeoutExceeded` to rig-log on timeout (tie-off preserved). Queue idle detection in event loop writes `QueueIdle` after 500ms of no events. 11 new unit tests + 11 new integration tests across `rig_log.rs` and `profile_timeout.rs`. Domain glossary updated with `Rig-log` term. 362 tests pass, clippy clean.

**PRD:** [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md)

Full details in [rig-log-notification-and-timeout.md](rig-log-notification-and-timeout.md).

### 27. Git Versioning — Automatic Commit History for Agent Work

**Status:** ✅ Complete
**Created:** 2026-06-13
**Completed:** 2026-06-14
**Goal:** Each knot run produces a git commit in the project root with structured message and tie-off body. Opt-out per-knot via `git-versioned: false` in frontmatter. Gracefully skips if not a git repo.

**Result:** `git_versioned: bool` field on `Knot` entity and `KnotFile` (parsed from `git-versioned` frontmatter, defaults `true`). `GitVersioningPort` trait + `MockGitVersioningPort`. `FileSystemGitVersioner` adapter uses `std::process::Command` to run `git` (no C dependency) — stages all changes with `git add -A`, commits with structured subject (`knot: <knot-id> — processed <strand-name> (<event-type>)`) and tie-off body. Graceful degradation: skips if not a git repo, git unavailable, or commit fails (non-fatal warnings). `ProcessStrand::execute()` calls git port after tie-off write when `knot.git_versioned` is `true`. Wired in composition root via `start_event_pipeline`. 17 new unit tests + 3 new integration tests in `tests/git_versioning.rs`. All 293+ tests pass.

**PRD:** [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md)

Full details in [git-versioning.md](git-versioning.md).

### 26. HTTP Observability Only — Remove Control Endpoints

**Status:** ✅ Complete
**Created:** 2026-06-13
**Completed:** 2026-06-13
**Goal:** Remove all control (POST/PUT/DELETE) endpoints from the HTTP interface, keeping only read-only observability (GET endpoints). Configuration (profiles, looms, knots) becomes file-first — skills write files directly, Knot's file watcher auto-discovers changes.

**Result:** 7 control endpoints removed (`POST /looms`, `DELETE /looms/{id}`, `POST /looms/{id}/knots`, `PATCH /looms/{id}/knots/{name}`, `DELETE /looms/{id}/knots/{name}`, `POST /profiles/{name}`, `DELETE /profiles/{name}`). Request types `RegisterLoomRequest`, `KnotRequest`, `ProfileRequest` removed. 3600+ lines of handler code and tests eliminated. `AgentProfile.body: Option<String>` added for profile markdown body. Skills updated to file-first approach. 317 tests pass (3 ignored). Version bumped to 0.3.0. ADR-006 documents the file-first approach; ADR-005 documents the skill integration testing strategy.

Full details in [http-observability-only.md](http-observability-only.md).

### 24. Tie-Off Output Rename and Knot File Cleanup

**Status:** ✅ Complete
**Created:** 2026-06-12
**Completed:** 2026-06-12
**Goal:** Rename `rig/output/` → `rig/tie-offs/`, tie-off filenames from `{strand}.output` → `{knot}-tie-off.md`, remove dead `tie-off-dir` from knot YAML parser, and add non-identified property detection with `.loom-log` warnings.

**Result:** `rig/output/` → `rig/tie-offs/`. Tie-off filenames: `{knot}-tie-off.md` (one per knot, append-mode). `RawFrontmatter` no longer accepts `tie-off-dir`. Unknown YAML properties emit `LoomEvent::KnotParseWarning` entries. `LoomRepository::scan()` now returns `(Vec<Loom>, Vec<String>)` with warnings. Domain glossary, agent skills, and all 48+ test path references updated. 331 tests pass.

Full details in [tieoff-output-rename-and-knot-cleanup.md](tieoff-output-rename-and-knot-cleanup.md).

### 23. Shared Agent Profiles

**Status:** ✅ Complete
**Created:** 2026-06-11
**Completed:** 2026-06-11
**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)
**Goal:** Allow multiple knots to reference shared agent profiles stored as `rig/profiles/{name}.md` files, with profile resolution at processing time so updates are picked up dynamically.

**Result:** 331 tests pass (262 unit + 61 integration). `AgentProfile` entity + parser, `KnotFile` extends with `agent-profile-ref` + mutual exclusivity validation, `AgentProfileRepository` port + file-system impl, `ProcessStrand` resolves profiles at processing time with inline overrides, CRUD endpoints for `/profiles`, knot handlers accept `agent_profile_ref`, 9 integration tests.

Full details in [shared-agent-profiles.md](shared-agent-profiles.md).

### 22. Notify Sender Leak Fix — Immediate Cascade Drain

**Status:** ⬜ Planned
**Created:** 2026-06-11
**Goal:** Split `NotifyEventSource` senders from callback state so channels close immediately on drop, removing the 5-second timeout safety net.

Full details in [notify-sender-leak-fix.md](notify-sender-leak-fix.md).

### 21. Static Output Paths and Log Timestamps

**Status:** ✅ Complete
**Created:** 2026-06-10
**Completed:** 2026-06-11
**Goal:** Make tie-off output paths and loom-log paths static (derived from loom/knot names under `rig/output/`), remove `tie-off-dir` from knot YAML, and add ISO 8601 timestamps to console logs and loom-log events.

**Result:** `tie_off_dir` removed from domain and KnotFile. Paths statically derived: `rig/output/{loom-id}/{knot-name}/{strand}.output` and `rig/output/{loom-id}/.loom-log`. ISO 8601 timestamps on all console logs and LoomEvent variants. 278 tests pass (196 lib + 82 integration, 1 ignored).

Full details in [static-output-paths-and-timestamps.md](static-output-paths-and-timestamps.md).

### 20. Knot Modification Observability and Path Resolution Consistency

**Status:** ✅ Complete
**Created:** 2026-06-08
**Completed:** 2026-06-15
**Goal:** Make `KnotModified` filesystem changes observable via loom-log (`LoomEvent::KnotUpdated`), log parse failures to stderr, and ensure path resolution is consistent between initial load and file-watcher events.

**Result:** Phase 0 (path resolution consistency) completed: `NotifyEventSource` now receives correct `project_root` (parent of rig directory) so relative `strand_dir` paths resolve identically to `FileSystemLoomRepository::scan()`. Full rename `base_dir` → `rig_dir` across all 7 source files + 17 test files to eliminate ambiguity between "rig directory" and "project root". Remaining phases (KnotUpdated loom-log, parse failure logging, integration test) remain Planned.

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
**Bugfix:** 2026-06-14 — multi-knot shared directory fanout (see [dpr-001](../dprs/dpr-001-multi-knot-watch-fanout.md))
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
