# Integration Test Overview

This directory contains Knot's integration test suite, organised by
feature area. Each module spins up the real composition root (or
exercises real adapters), in the process, or against the built binary —
and verifies end-to-end behaviour.

Since plan 083 (consolidated service log), the retired `.loom-log` /
`.rig-log` JSONL files are **no longer asserted on anywhere**: binary
tests pin the `[KNOT][EVENT]` / `[KNOT][STATE]` stderr lines (see
`consolidated_log.rs`) and in-process tests assert on the durable
surfaces — `state.json`, tie-off file sections, and event-queue files.

## Test Modules

| Module | Tests | Scope |
|--------|-------|-------|
| [`adapters.rs`](adapters.rs) | 30 | Adapter contract tests — one suite per outbound adapter (state writer, tie-off sink, event dispatcher/store, profile repo, model registry, …) |
| [`agent_integration.rs`](agent_integration.rs) | 17 | External agent CLI invocation — stub `pi` CLI happy path, agent error capture in tie-offs and run activity |
| [`composition.rs`](composition.rs) | 6 | Composition root wiring — verifies all hexagonal layers are connected correctly (no binary spawned) |
| [`consolidated_log.rs`](consolidated_log.rs) | 4 | **Plan 083** — binary-level: the full `[EVENT]` run sequence, no `.rig-log`/`.loom-log` files, `state.json` baseline + change-driven rewrite (idle ticks cause no write), burst-2-strand `queue+`/`queue-` deltas, and `knot step` baseline + delta lines |
| [`discovery.rs`](discovery.rs) | 10 | Loom discovery from the rig directory, filtering of non-loom directories, and `run_startup` registration |
| [`event_enforcement.rs`](event_enforcement.rs) | 16 | Event enforcement — producer/consumer intent pairing and enforcement violations |
| [`event_fanout.rs`](event_fanout.rs) | 8 | Producer fan-out to consumer processing (one producer event → several consumer tie-offs) |
| [`filesystem_interface.rs`](filesystem_interface.rs) | 3 | Filesystem create/list/roundtrip — minimal, focused |
| [`generic_task_management.rs`](generic_task_management.rs) | 10 | Channel-cascade shutdown pattern (generic task-tree teardown) |
| [`git_versioning.rs`](git_versioning.rs) | 15 | Git versioning of tie-offs and rig state |
| [`late_removal.rs`](late_removal.rs) | 19 | Late-removal (at-least-once) queue semantics — plan 073 phase 2 (tie-off-section and queue-file assertions) |
| [`model_aliases.rs`](model_aliases.rs) | 5 | Model aliases (069 phase 4) — `model-ref` resolution through the full composition |
| [`multi_loom.rs`](multi_loom.rs) | 9 | Multi-loom isolation (no cross-contamination) and per-knot source directory separation; per-loom tie-off isolation |
| [`persistent_queue.rs`](persistent_queue.rs) | 8 | Persistent (disk-backed) event queue — enqueue/drain/crash semantics |
| [`pipeline.rs`](pipeline.rs) | 19 | Full event pipeline: Notify → debounce → ProcessStrand → tie-off; strand lifecycle (create/modify/delete) |
| [`profile_timeout.rs`](profile_timeout.rs) | 11 | Agent timeout handling (total deadline) through profiles |
| [`queue_identity.rs`](queue_identity.rs) | 10 | Queue entry identity self-heal — plan 075 phase 2 incident reproduction (full composition) |
| [`rig_cli.rs`](rig_cli.rs) | 10 | `knot` binary CLI: rig switching/sharing, startup migration of a legacy layout (migrated logs are inert), `knot stop` |
| [`rig_discovery.rs`](rig_discovery.rs) | 8 | Rig discovery (domain layer) — `*-rig` scanning, explicit names |
| [`rig_log.rs`](rig_log.rs) | 10 | Operational-event recording on `RigLogPort` — `TimeoutExceeded` on deadline breaches, no event on success/non-timeout failure (in-memory since plan 083) |
| [`session_resume.rs`](session_resume.rs) | 16 | Session-resume retry on invocation failure |
| [`smoke.rs`](smoke.rs) | 9 | Smoke tests — full composition with a mock agent via `cli_path` injection |
| [`step.rs`](step.rs) | 17 | `knot step` lifecycle — plan 073 phase 4; binary-level `[EVENT]`/`[STATE]` line assertions plus tie-off-section completion tracking |
| [`thinking_level.rs`](thinking_level.rs) | 2 | Thinking-level pass-through (074 phase 5) |
| [`tie_off.rs`](tie_off.rs) | 17 | Tie-off output — append-mode history, markdown section structure, producer/consumer sections |

## Shared Infrastructure

| Module | Purpose |
|--------|---------|
| [`helpers.rs`](helpers.rs) | Shared test fixtures: knot/loom YAML creation, mock/stub agents, mock ports, runtime-root derivation, and event-file counting |

## Notes

- Binary-level tests (`consolidated_log.rs`, `step.rs`, `rig_cli.rs`)
  spawn the built binary in a temp directory with `KNOT_TEST_DEBOUNCE_MS`
  / `KNOT_TEST_CHECK_MS` / `KNOT_STATE_WRITE_MS` speed-ups and a mock
  `pi` injected via `KNOT_TEST_CLI_PATH`, and capture stdout+stderr.
- In-process tests build the real composition root from
  `knot::build_app_context` / `knot::run_startup` (2-arg since plan
  083) with in-memory run-activity adapters, and assert on durable
  surfaces only.
- All integration tests run in parallel by default via `cargo test` —
  each test creates its own temp directory.
