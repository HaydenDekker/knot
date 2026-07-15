# Master Plan — Project Index

> **Last Updated:** 2026-07-15

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
| 60 | [Pending Event Visibility](060-pending-event-visibility/060-pending-event-visibility-plan.md) | ✅ Complete | 2026-07-15 |
| 59 | [Tie-Off Event Enforcement](059-tie-off-event-enforcement/tie-off-event-enforcement-plan.md) | ✅ Complete | 2026-07-14 |
| 58 | [Loom-Level Event Subscriptions](058-loom-level-events/loom-level-events-plan.md) | ✅ Complete | 2026-07-11 |
| 57 | [Agent Event Format — Markdown Code Blocks](057-event-format-markdown-blocks/event-format-markdown-blocks-plan.md) | ✅ Complete | 2026-07-11 |
| 56 | [Strand Event URI](056-strand-event-uri/strand-event-uri-plan.md) | ✅ Complete | 2026-07-10 |
| 53 | [Integration Test Strategy](053-integration-test-strategy/integration-test-strategy-plan.md) | ✅ Complete | 2026-07-01 | (completed 2026-07-03)
| 52 | [Flatten Tie-Off Paths](052-flat-tie-off-paths/flat-tie-off-paths-plan.md) | ✅ Complete | 2026-07-01 |
| 51 | [Filter Final Response in Pi JSON Adapter](051-pi-json-final-response-filter/pi-json-final-response-filter-plan.md) | ✅ Complete | 2026-07-01 |
| 50 | [Strand Queue Visibility in State](strand-queue-in-state.md) | ✅ Complete | 2026-06-30 |
| 49 | [Split `process_strand.rs` Tests into Isolated Module](process-strand-test-extraction.md) | ✅ Complete | 2026-06-29 |
| 48 | [Split `usecases.rs` into Isolated Modules](usecases-refactor.md) | ✅ Complete | 2026-06-29 |
| 47 | [Session Resume on Invocation Failure](session-resume-on-invocation-failure.md) | ✅ Complete | 2026-06-28 |
| 46 | [JSON-based Agent Adapter](agent-json-adapter.md) | ✅ Complete | 2026-06-27 |
| 45 | [Intent-based Event Routing](intent-based-event-routing.md) | ✅ Complete | 2026-06-25 | (completed 2026-07-09)
| 44 | [Fix `unwatch()` Removing Watchers for Other Knots](bugfix-unwatch-removes-wrong-watchers.md) | ✅ Complete | 2026-06-24 |
| 43 | [Simplify Prompts — Move Prompt Text to Markdown Body](simplify-prompt-in-body.md) | ✅ Complete | 2026-06-24 |
| 42 | [Strand Missing File Handling](strand-missing-file-handling.md) | ✅ Complete | 2026-06-24 |
| 41 | [Tie-Off Context Extraction for Agent Processing](tie-off-context-extraction.md) | ✅ Complete | 2026-06-22 |
| 40 | [Remove `input-bundling` from Prompt Template](remove-input-bundling.md) | ✅ Complete | 2026-06-20 |
| 39 | [Accept All Text Files as Strands](accept-all-text-strands.md) | ✅ Complete | 2026-06-19 |
| 38 | [Removal of HTTP Interface — Full File-First](removal-of-http-interface.md) | ✅ Complete | 2026-06-18 |
| 37 | [User Documentation and Documentation Skill](user-documentation.md) | ✅ Complete | 2026-06-18 |
| 36 | [Explicit Pi Session Title](pi-session-title.md) | ✅ Complete | 2026-06-17 |
| 35 | [Rig Switching and Sharing](rig-switching-and-sharing.md) | ✅ Complete | 2026-06-17 |
| 34 | [Strand Directory Auto-Creation](strand-dir-auto-create.md) | ✅ Complete | 2026-06-17 |
| 33 | [Queue Event Dedup — Prevent Duplicate Strand Processing](queue-event-dedup.md) | ✅ Complete | 2026-06-16 |
| 32 | [Simplify Agent Invocation — Remove --system-prompt](simplify-agent-invocation.md) | ✅ Complete | 2026-06-16 |
| 31 | [Agent Profile Skills](agent-profile-skills.md) | ⬜ Planned | 2026-06-16 |
| 30 | [Context Management — Slim Agent Prompt and Tie-Off Headers](context-management.md) | ✅ Complete | 2026-06-15 |
| 29 | [Auto-Discovery Reliability Fixes](auto-discovery-reliability.md) | ✅ Complete | 2026-06-15 |
| 28 | [Rig-Log Notification and Timeout Handling](rig-log-notification-and-timeout.md) | ✅ Complete | 2026-06-14 |
| 27 | [Git Versioning — Automatic Commit History for Agent Work](git-versioning.md) | ✅ Complete | 2026-06-13 |
| 26 | [HTTP Observability Only — Remove Control Endpoints](http-observability-only.md) | ✅ Complete | 2026-06-13 |
| 22 | [Notify Sender Leak Fix — Immediate Cascade Drain](notify-sender-leak-fix.md) | ⬜ Planned | 2026-06-11 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | 🟡 In Progress | 2026-06-08 |

---

_Overview sections for active and recently completed plans go here._

### 60. Pending Event Visibility

**Status:** ✅ Complete
**Created:** 2026-07-15
**Completed:** 2026-07-15
**Goal:** Give producer knots visibility into previously dispatched events so they can avoid emitting duplicative events on re-processing (session resume, retry, re-processing the same strand).

**Result:** `ContextProvider` trait and `BuildContext` struct defined in domain. `AgentEventsContextProvider` implementation scans event dispatch directories for pending events and injects them into the prompt. Event format now documents `timestamp` as a required field alongside `event` and `description`. Dispatch adapter prefers agent's timestamp from payload. "Do not edit" guidance added to prompt. 18 new tests, 668 total pass. Version bumped to 0.28.0.

**Design document:** [DPR-002: Context Provider Pattern and Pending Event Visibility](../dprs/dpr-002-context-provider-and-pending-events.md)

Full details in [060-pending-event-visibility/060-pending-event-visibility-plan.md](060-pending-event-visibility/060-pending-event-visibility-plan.md).

### 59. Tie-Off Event Enforcement

**Status:** ⬜ Planned
**Created:** 2026-07-14
**Goal:** Detect when agents instructed to emit tie-off events fail to do so, log the failure via `KnotEventsMissing` in the loom-log, and re-enter the session with a follow-up prompt to remind the agent to provide events.

Full details in [059-tie-off-event-enforcement/tie-off-event-enforcement-plan.md](059-tie-off-event-enforcement/tie-off-event-enforcement-plan.md).

### 56. Strand Event URI

**Status:** ✅ Complete
**Created:** 2026-07-10
**Completed:** 2026-07-10
**Goal:** Replace `listens-for` array with `strand-dir: "event:<producer>:<EventId>"` URI scheme so each knot has exactly one input direction, eliminating fan-in and the `Intent` struct.

**Result:** `StrandSource` enum (`Filesystem`/`EventUri`) replaces `listens_for: Vec<Intent>`. `strand-dir` now accepts plain paths or `event:<producer>:<EventId>` URIs. `event-description` field added for producer prompt injection. `build_listener_context()` produces improved format with markdown heading, per-event-type blocks, and `event: None` signal. Tie-off parser returns `Vec<AgentEvent>` — multiple event types per tie-off. `Intent`, `matches_intent()`, `default_listens_for()` fully removed. Unified `ensure_strand_source_watch()` replaces separate watcher functions. `From<KnotFile> for Knot` canonical factory, `KnotBuilder` test helper. 598 tests pass, clippy clean. Version bumped to 0.24.0.

Full details in [056-strand-event-uri/strand-event-uri-plan.md](056-strand-event-uri/strand-event-uri-plan.md).

### 53. Integration Test Strategy

**Status:** ✅ Complete
**Created:** 2026-07-01
**Completed:** 2026-07-02
**Goal:** Adopt hexagonal test strategy: application tests against mock ports, one adapter test per adapter (real I/O + `tempfile`), two composition smoke tests (`cli_path` injection). Eliminate `TEST_MUTEX`, process-global env vars, and full-runtime-per-test patterns. Target: ~100 tests, <30s total, fully parallel.

**Result:** Phases 0-6 implemented hexagonal test architecture. Phase 7 verification: 746 tests pass (0 failures), identical results under `--test-threads=4`, no new clippy warnings. `TEST_MUTEX`, `std::env::set_var("PATH")`, and `KNOT_TEST_CLI_PATH` all confirmed absent from test code. Lib tests dropped from 100s to 1.1s after fixing retry delay env vars. Flaky `test_session_resume_delay_between_retries` removed (redundant with unit test). Wall clock 54s — target of <30s not met due to remaining unmigrated integration suites (auto_discovery: 15s, 6 suites at ~5s each). Test count of 746 exceeds target of ~100 — many pre-plan integration test files remain unmigrated.

**ADR:** [ADR-011: Hexagonal Test Strategy](../adrs/adr-011-hexagonal-test-strategy.md)

Full details in [053-integration-test-strategy/integration-test-strategy-plan.md](053-integration-test-strategy/integration-test-strategy-plan.md).

### 52. Flatten Tie-Off Paths

**Status:** ✅ Complete
**Created:** 2026-07-01
**Completed:** 2026-07-01
**Goal:** Remove the intermediate `{knot-name}` subdirectory from tie-off paths, flattening them to `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`, freeing subdirectories for event capture.

**Result:** `derive_tieoff_path()` simplified from 3-level to 2-level nesting. All code paths, unit tests, integration tests, domain glossary, user docs, and agent skills updated to flat structure. Version bumped to 0.22.0. Migration entry added to knot-update skill. PATH race condition fix: serialisation locks added to 7 test suites (27 test functions). 199 integration tests pass, 475 unit tests pass.

Full details in [052-flat-tie-off-paths/flat-tie-off-paths-plan.md](052-flat-tie-off-paths/flat-tie-off-paths-plan.md).

### 51. Filter Final Response in Pi JSON Adapter

**Status:** ✅ Complete
**Created:** 2026-07-01
**Completed:** 2026-07-01
**Goal:** Fix `PiJsonAgentRunner` to extract only the agent's final response text, not intermediate tool-use messages. When Pi uses tools, multiple assistant messages are produced — intermediate ones with `stopReason: "toolUse"` and the final with `stopReason: "stop"` or `"length"`. Previously all assistant text was concatenated.

**Result:** `message_end` handler removed (work fully covered by `agent_end` with complete message array). `agent_end` handler filters by `stopReason` — only `"stop"` and `"length"` produce response text; `"toolUse"`, `"error"`, `"aborted"` are excluded. 3 existing test fixtures updated with `stopReason: "stop"`. 1 existing test (`message_end`) changed to assert empty response. 4 new tests covering tool-use exclusion, length inclusion, error exclusion, and multi-turn filtering. All 476 tests pass, clippy clean. Bugfix — no version bump.

Full details in [051-pi-json-final-response-filter/pi-json-final-response-filter-plan.md](051-pi-json-final-response-filter/pi-json-final-response-filter-plan.md).

### 50. Strand Queue Visibility in State

**Status:** ⬜ Planned
**Created:** 2026-06-30
**Goal:** Add `strand_queue` array to `rig/state.json` showing all pending strand events with file path, loom/knot IDs, event type, and queued timestamp.

Full details in [strand-queue-in-state.md](strand-queue-in-state.md).

### 49. Split `process_strand.rs` Tests into Isolated Module

**Status:** ✅ Complete
**Created:** 2026-06-29
**Completed:** 2026-06-29
**Goal:** Consolidate ~3,358 lines of inline tests from `process_strand.rs` (3,862 lines total) — remove dead-code stubs, rename phase-numbered modules by concern, consolidate duplicated mocks/helpers into `test_fixtures.rs`, and split execution tests into focused sub-modules. Pure structural refactor — zero behaviour change.

**Result:** 7 test modules renamed to describe what they test. 2 empty dead-code stubs removed. Shared helpers (`TrackingTieOffSink`, `TrackingAgentRunner`, `build_knot_with_profile`, `default_profile`) consolidated into `test_fixtures.rs`. Duplicate local mock definitions eliminated. `execution_tests` split into `execution_tests`, `execution_deleted_tests`, and `session_resume_tests`. Net change: −669 lines (3,862 → 3,193). 461 unit tests pass, 38 process_strand tests pass with `--test-threads=1`. No version bump — pure structural refactor.

Full details in [process-strand-test-extraction.md](process-strand-test-extraction.md).

### 47. Session Resume on Invocation Failure

**Status:** ✅ Complete
**Created:** 2026-06-28
**Completed:** 2026-06-28
**Goal:** Automatically resume Pi sessions from where they left off after invocation failure (timeout, network error) using `--session-id`, up to 10 retries or until the profile's overall timeout budget is exhausted. Appends "please continue" to the session on retry. 10-second delay between retries for network recovery.

**Result:** `SessionResumed` variant on `LoomEvent` + `is_session_resumable()` helper in ports. `session_resume.rs` module with `execute_with_resume()` retry loop (MAX_RETRIES=10, RETRY_DELAY=10s, MIN_REMAINING=5s). Wired into `ProcessStrand::execute()` — retry is transparent to the outer flow (tie-off write, git commit, KnotCompleted log all proceed normally). 23 new tests: 5 unit (Phase 0) + 13 unit (session_resume module) + 3 ProcessStrand integration + 2 integration test file (7 tests). 432 unit tests pass, clippy clean. Version bumped to 0.20.0.

**PRD:** [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md)

Full details in [session-resume-on-invocation-failure.md](session-resume-on-invocation-failure.md).

### 44. Fix `unwatch()` Removing Watchers for Other Knots

**Status:** ✅ Complete
**Created:** 2026-06-24
**Completed:** 2026-06-26
**Goal:** Fix `unwatch()` removing all watcher entries for a path when only a single knot's entry should be removed — breaking shared strand directory scenarios where multiple knots watch the same directory.

**Result:** `unwatch_with_type()` method added to `EventSource` trait with default impl delegating to `unwatch()` for backward compat. `NotifyEventSource::unwatch_with_type()` removes only the matching `(path, WatchType)` pair using `watch_types_equal`, calls `notify::unwatch()` only when no other entries remain for the path. Callers in `handle_knot_modified` and `handle_knot_deleted` changed to use `unwatch_with_type`. 2 new unit tests in `event_source.rs` + 1 new integration test (`multi_knot_shared_directory_unwatch_does_not_remove_other_watch`). Version bumped to 0.18.1.

Full details in [bugfix-unwatch-removes-wrong-watchers.md](bugfix-unwatch-removes-wrong-watchers.md).

### 46. JSON-based Agent Adapter

**Status:** ✅ Complete
**Created:** 2026-06-27
**Completed:** 2026-06-27
**Goal:** Add a JSON-L subprocess adapter that captures session IDs and token usage from Pi invocations. Rig config selects adapter via `agent_adapter` enum (`pi-stdio` or `pi-json`) — no `cli_path`/`cli_args` in config.

**Result:** `AgentInvocationMetadata` + `TokenUsage` structs in ports, `session_id` on `PortError::Timeout`/`AgentExecutionFailed`. `AgentAdapter` enum replaces `cli_path`/`cli_args` in `RigAgentConfig`. `PiJsonAgentRunner` parses JSON-L line-by-line for session ID + token usage. `SubprocessAgentRunner` renamed to `PiStdioAgentRunner`. `run_startup()` auto-creates `.workspace-agent-config.yaml` on first boot. 3 integration tests + 14 unit tests. 612+ tests pass. Version bumped to 0.19.0.

**ADR:** [ADR-009: Agent-Specific Adapters](../adrs/adr-009-agent-specific-adapters.md)

**PRD:** [Demand Control — Concurrency, Throughput and Service Tuning](../prds/prd-demand-control.md)

Full details in [agent-json-adapter.md](agent-json-adapter.md).

### 45. Intent-Based Event Routing

**Status:** ✅ Complete
**Created:** 2026-06-25
**Completed:** 2026-07-09
**Goal:** Add first-class agent-to-agent events: consumer knots declare `listens-for` intents in frontmatter, Knot injects event instructions into producer prompts, and dispatches matching structured events to consumer tie-off directories.

**Result:** `Intent` struct (`target_knot`, `event_id`, `event_description`) on `KnotFile`/`Knot`. `AgentEvent` struct with payload map on `TieOff`. `EventMetadata` struct (`event_id`, `source_knot`, `original_strand`) for observability. `EventDispatcherPort` trait + `FileSystemEventDispatcher` adapter. `build_listener_context()` injects event instructions per-invocation. `extract_agent_events()` parses structured events from tie-off content. `matches_intent()` matches by `event-id` + `target-knot`. Wired into `ProcessStrand::execute()` — after successful completion, events are parsed, matched, and dispatched to consumers. `EventsDispatched` loom-log variant for traceability. 669 tests pass, 0 failures. Version bumped to 0.23.0.

Full details in [intent-based-event-routing.md](intent-based-event-routing.md).

### 43. Simplify Prompts — Move Prompt Text to Markdown Body

**Status:** ⬜ Planned
**Created:** 2026-06-24
**Goal:** Remove `profile-prompt` and `prompt-template.instructions` from YAML frontmatter; use the markdown body as the prompt text directly.

Full details in [simplify-prompt-in-body.md](simplify-prompt-in-body.md).

### 42. Strand Missing File Handling

**Status:** ✅ Complete
**Created:** 2026-06-24
**Completed:** 2026-06-24
**Goal:** Silently skip known temp files (e.g. `sed -i` macOS temp files) and log unknown missing files, avoiding spurious "File not found" errors in the loom-log.

**Result:** `is_known_temp_file()` in `src/domain/temp_file.rs` detects sed temp files (filename: `sed` + 7 chars). `LoomEvent::StrandSkipped` variant in domain events for unknown missing files. File existence check in `ProcessStrand::execute()` after text-file check — known temp files skip silently (debug log only), unknown missing files log `StrandSkipped` + console warning. Deleted events unaffected. 5 new unit tests in `phase2_file_existence_tests`, 8 tests in `temp_file` module, 2 integration tests in `pipeline.rs`. 12 existing tests fixed for real temp files. 586+ tests pass. Version bumped to 0.17.0.

Full details in [strand-missing-file-handling.md](strand-missing-file-handling.md).

### 41. Tie-Off Context Extraction for Agent Processing

**Status:** ✅ Complete
**Created:** 2026-06-22
**Completed:** 2026-06-22
**Goal:** Parse tie-off files into per-strand sections, extract the last N entries for the specific strand, and inject scoped history into the agent prompt for deletion events (replacing the `@file` reference that fails on deleted files).

**Result:** `TieOffSection` struct + `parse_sections()` / `extract_last_n()` in `src/domain/tieoff_parser.rs` (line-by-line state machine parser, no regex). `ProcessStrand::execute()` integrates parser for Deleted events — skips `@file`, injects deletion notice + last 5 per-strand entries from tie-off. Created/Modified events unchanged. 9 unit tests in domain layer, 5 unit tests in application layer, 3 integration tests in `tests/pipeline.rs`. Path-mismatch bug fixed during Phase 2. 366 tests pass. Version bumped to 0.16.0.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [tie-off-context-extraction.md](tie-off-context-extraction.md).

### 40. Remove `input-bundling` from Prompt Template

**Status:** ✅ Complete
**Created:** 2026-06-20
**Completed:** 2026-06-20
**Goal:** Remove the `input-bundling` property from `PromptTemplate` — it was required in knot YAML frontmatter but had no runtime effect. Only `full-file` ever shipped and is always the behaviour.

**Result:** `input_bundling` field removed from `PromptTemplate` struct, `RawPromptTemplate`, parsing logic, and all test fixtures across domain, application, outbound adapters, and integration tests. Docs, skills, and rig demo files updated to remove the property. Knot files that still contain `input-bundling` parse successfully with an unknown-property warning. 23 files changed, -96 lines net. All tests pass. Version bumped to 0.15.0.

Full details in [remove-input-bundling.md](remove-input-bundling.md).

### 39. Accept All Text Files as Strands

**Status:** ✅ Complete
**Created:** 2026-06-19
**Completed:** 2026-06-19
**Goal:** Extend strand input so knots can operate on any text file (.rs, .json, .py, .txt, etc.) — not just `.md`.

**Result:** `.md` extension filter removed from `NotifyEventSource`. `is_text_file()` utility in `adapters/outbound/content_inspector.rs` uses `content_inspector` crate (null-byte heuristic on first 8KB). Binary files produce `LoomEvent::StrandIgnored` in loom-log + stderr warning, then skip agent execution. Deleted events bypass text check (file is gone). 5 new unit tests in `event_source.rs`, 5 in `usecases.rs`, 2 integration tests in `pipeline.rs`. 354 tests pass. Version bumped to 0.14.0.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [accept-all-text-strands.md](accept-all-text-strands.md).

### 38. Removal of HTTP Interface — Full File-First

**Status:** ✅ Complete
**Created:** 2026-06-18
**Completed:** 2026-06-19
**Goal:** Remove the Axum HTTP server entirely and replace all state observation with `rig/state.json` written on a 5-second poll cycle.

**Result:** HTTP server and all inbound adapter code removed (~7000 lines). `axum`, `utoipa`, `utoipa-swagger-ui`, `tower` dependencies removed. `RigState` domain type + `StateWriter` background task writes `rig/state.json` atomically every 5 seconds. All skills updated to read `rig/state.json`. All integration tests rewritten from HTTP to file-based polling. 546 tests pass. Version bumped to 0.13.0. ADR-008 documents the decision.

Full details in [removal-of-http-interface.md](removal-of-http-interface.md).

### 37. User Documentation and Documentation Skill

**Status:** ✅ Complete
**Created:** 2026-06-18
**Completed:** 2026-06-18
**Goal:** Create user-facing documentation from existing project artifacts (skills, glossary, PRDs, completed plans) and package the extraction methodology into a reusable `project-user-documentation` skill.

**Result:** 11 user-facing docs created in `docs/`: getting-started, concepts, 3 configuration guides (profiles, knots, rig-structure), 2 workflow tutorials (review, file-generation), API reference, troubleshooting guide, design guide, and release notes. `project-user-documentation` skill (393 lines) created at `.agents/skills/project-user-documentation/SKILL.md` and published globally. README updated with documentation index. Documentation-only — no version bump needed.

Full details in [user-documentation.md](user-documentation.md).

### 36. Explicit Pi Session Title

**Status:** ✅ Complete
**Created:** 2026-06-17
**Completed:** 2026-06-17
**Goal:** Add `--name` CLI flag to pi invocation so each session gets a unique, descriptive resume title derived from knot ID and strand filename.

**Result:** `--name` appended to CLI args in `ProcessStrand::execute()` with title format `{knot-id} triggered by {event-type} on {strand-filename}` (e.g. `plan-architect triggered by Modified on 004-manifest-resources.md`). Edge case guarded with `unwrap_or_default()`. 6 new tests (1 in `subprocess.rs`, 5 in `usecases.rs`) covering flag passthrough, title formats for Created/Modified/Deleted events, uniqueness per strand, and prompt content regression guard. 325 tests pass. Version bumped to 0.12.0.

Full details in [pi-session-title.md](pi-session-title.md).

### 35. Rig Switching and Sharing

**Status:** ✅ Complete
**Created:** 2026-06-17
**Completed:** 2026-06-17
**Goal:** Enable switching between multiple rigs on the same project and packaging rigs for sharing with colleagues by distributing loom definitions (excluding derived state).

**Result:** CLI parsing via `std::env::args()` — no external crate needed. `knot` (no args) auto-discovers `*-rig` directories: zero matches creates `rig/`, one match uses it, multiple refuses with usage hint. `knot <rig-name>` uses named rig. `knot share <rig-name>` packages looms + profiles into `.zip` via `zip` crate (excludes tie-offs, logs, config). `RigDiscovery` domain enum + `discover_rigs()` pure function. `AppConfig::with_rig_dir()` convenience constructor. 13 new tests (8 unit + 10 integration, some shared across files). 395+ tests pass. Version bumped to 0.11.0.

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
