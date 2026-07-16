# Master Plan — Project Index

> **Last Updated:** 2026-07-17 (plan 64 completed, old plans purged)

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
| 64 | [Local Time Timestamps](064-local-timestamps/local-timestamps-plan.md) | ✅ Complete | 2026-07-17 |
| 63 | [Spurious Delete Suppression](063-spurious-delete-suppression/spurious-delete-suppression-plan.md) | 📝 Draft | 2026-07-16 |
| 62 | [Decompose `ProcessStrand::execute()`](062-process-strand-decomposition/process-strand-decomposition-plan.md) | ✅ Complete | 2026-07-16 |
| 61 | [Consolidate `build_process_strand` Test Helpers](061-consolidate-build-process-strand/consolidate-build-process-strand-plan.md) | ✅ Complete | 2026-07-15 |
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
| 31 | [Agent Profile Skills](agent-profile-skills.md) | ⬜ Planned | 2026-06-16 |
| 22 | [Notify Sender Leak Fix — Immediate Cascade Drain](notify-sender-leak-fix.md) | ⬜ Planned | 2026-06-11 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | 🟡 In Progress | 2026-06-08 |

---

_Overview sections for active and recently completed plans go here._

### 64. Local Time Timestamps

**Status:** ✅ Complete
**Created:** 2026-07-17
**Completed:** 2026-07-17
**Goal:** Replace UTC timestamps with local-time timestamps (ISO 8601 with timezone offset) across all Knot output — logs, loom-logs, event files, tie-offs, and state.

**Result:** `chrono` crate added as dependency. `format_timestamp()` in `logging.rs` now uses `chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z")` producing timestamps like `2026-07-17T14:30:00+01:00`. Duplicate `format_timestamp()` and `days_to_ymd()` removed from `tieoff_sink.rs` (delegates to `logging`). All doc comments updated from "UTC" to "local time". Prompt template updated to reference local time. 2 new unit tests. Version bumped to 0.29.0.

Full details in [064-local-timestamps/local-timestamps-plan.md](064-local-timestamps/local-timestamps-plan.md).

### 63. Spurious Delete Suppression

**Status:** 📝 Draft
**Created:** 2026-07-16
**Goal:** Suppress spurious DELETE events caused by atomic file rewrites (truncate+write) using a configurable 5-second suppression window in the debounce engine.

Full details in [063-spurious-delete-suppression/spurious-delete-suppression-plan.md](063-spurious-delete-suppression/spurious-delete-suppression-plan.md).

### 61. Consolidate `build_process_strand` Test Helpers

**Status:** ✅ Complete
**Created:** 2026-07-15
**Completed:** 2026-07-16
**Goal:** Consolidate 8 duplicate `build_process_strand` helper functions across integration test files into a single shared `ProcessStrandBuilder` with fluent setters.

**Result:** `ProcessStrandBuilder` in `tests/helpers.rs` with fluent API (`with_profile()`, `with_looms()`, `with_tracking_git()`, `with_tracking_event_dispatcher()`, `with_tracking_file_checker()`). `ProcessStrandResult` struct with typed fields and optional tracking ports. All 8 local `build_process_strand` functions removed from `tie_off.rs`, `rig_log.rs`, `agent_integration.rs`, `profile_timeout.rs`, `session_resume.rs`, `git_versioning.rs`, `event_enforcement.rs`, and `pipeline.rs`. All 909 tests pass, clippy clean. No version bump — pure test refactor.

Full details in [061-consolidate-build-process-strand/consolidate-build-process-strand-plan.md](061-consolidate-build-process-strand/consolidate-build-process-strand-plan.md).

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
