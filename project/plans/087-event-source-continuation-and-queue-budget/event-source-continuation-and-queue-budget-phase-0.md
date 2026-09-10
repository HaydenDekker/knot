# Phase 0: Batch Total Execution Budget (Bound 2)

**Plan:** [Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget](087-event-source-continuation-and-queue-budget-plan.md)

## Checklist

- [x] `ResolvedExecution` (`src/application/usecases/process_strand_helpers.rs:19`): replace `batch_deadline_epoch: Option<u64>` with `budget_secs: Option<u64>` + `batch_start_epoch: Option<u64>`; add `dequeue_epoch: Option<u64>` (this hop's start) and keep `incoming_continuations: u32`. Update the doc comment to describe the queue-wait-exempt budget
- [x] `read_continuation_stamps` (`~729`): return `(continuations, budget_secs, batch_start_epoch)`; read the new `budget-secs` + `batch-start-epoch` front-matter keys, with a **compatibility shim** falling back to `batch-deadline-epoch` (derive `budget_secs = deadline - now` at read time, `batch_start_epoch = None`) for any pre-087 continuation files still on disk
- [x] Dequeue check in `resolve_config_and_build` (`~214-235`): set `effective_timeout = Some(Duration::from_secs(budget_secs))` **directly** (drop the `deadline - now` subtraction); record `dequeue_epoch` (Unix seconds, captured here at dequeue time) and thread it into `ResolvedExecution`; if `budget_secs < MIN_REMAINING_SECS` → `degenerate_deadline_tieoff`
- [x] `degenerate_deadline_tieoff` (`~762`): take `budget_secs` + `batch_start_epoch` + `dequeue_epoch` for its `BatchIncomplete` record; keep the `Failed` deferral outcome
- [x] `dispatch_self_continuation` (`~836`): replace the `deadline_epoch = incoming_deadline.unwrap_or(now + profile_secs)` computation with the budget decay — first handoff: `budget = profile_secs - execution_secs`, `start = now`; continuation hop: `budget = incoming_budget - execution_secs`, `start = incoming_start` (inherited, never recomputed); where `execution_secs = now - dequeue_epoch` (wall-clock, dequeue → handoff); stamp `budget-secs: budget` + `batch-start-epoch: start` onto the continuation file front-matter (replacing the `batch-deadline-epoch` line at `~909`)
- [x] `LoomEvent::TasksIncomplete` / `BatchIncomplete` (`src/domain/events.rs`): carry `budget_secs` + `batch_start_epoch` (replacing/alongside `deadline_epoch`); update all constructors (`~433`, `~447-451`, `~800`, `~819`) and the `~433` `handle_success` call site
- [x] Update the 082 system-event payload + the `[task-loop] handoff` service-log line to print `remaining=<B>'s` (budget) alongside `hop=N/10`
- [x] TDD: a unit test pinning the deque-timeout shape (a continuation with `budget-secs: 1200` gets a 1200s runner timeout regardless of elapsed wall-clock) is written **red** first, then the implementation turns it green; a shim test confirms a `batch-deadline-epoch`-only file still resolves a budget
- [x] `cargo build` clean; `cargo clippy` clean; `cargo test` green

## Deviations

- **Budget decay lives in a new helper `stamp_continuation_budget`, called from `handle_success`** (before the `LoomEvent::TasksIncomplete` append) rather than inside `dispatch_self_continuation`. The checklist's decay semantics are implemented verbatim (first handoff vs continuation hop, `execution_secs = now − dequeue_epoch`, `saturating_sub`); the extraction is required so the loom-event payload and the service-log line carry the **same stamped budget** the continuation file receives (the event append precedes the dispatch in the existing order — phase 2 preserves that order for the unsuppressed path). `dispatch_self_continuation` now takes the stamped `budget_secs` + `batch_start_epoch` as parameters.
- **Degenerate note/error wording** updated from "batch deadline exhausted" to "batch execution budget exhausted" (the `BatchIncomplete` `reason` string stays `"deadline"` — the plan keeps the reason taxonomy for this phase).
- **`#[allow(clippy::too_many_arguments)]`** added to `degenerate_deadline_tieoff` (9/7) and `dispatch_self_continuation` (8/7), matching the project's existing convention (`live_output.rs`; the same lint is already tolerated on `handle_success` 11/7 in the same file and on `execute_with_resume` 14/7). Net clippy diff vs baseline: **zero new warning bodies**.
- The `service_log.rs` renderer references `crate::application::session_resume::MAX_CONTINUATIONS` for the `hop=N/10` format (adapters already reference `crate::application` elsewhere in this codebase; the constant stays single-sourced).

## Discoveries

- The "082 system-event payload" for `TasksIncomplete` / `BatchIncomplete` is the **`LoomEvent` fields rendered on the `[KNOT][EVENT]` line** — the 086 implementation emitted these as loom events (no `emit_system` call exists for them; no system-event payload table entry either). Phase 0 therefore updates the `LoomEvent` variants + the `render_loom_event_line` renderers + the `log_strand_event` line.
- `LoomEvent::TasksIncomplete` now carries the **stamped (post-hop) budget** — the exact value the continuation file gets — not the incoming budget; a reviewer can therefore reconstruct the chain hop-by-hop from the events alone.
- The `caps`-suppression `BatchIncomplete` now carries the budget the suppressed continuation would have stamped.
- Tests added: 6 unit tests in `process_strand_helpers::tests` (`read_continuation_stamps` ×3, `stamp_continuation_budget` ×3) and 3 execute()-level tests in `process_strand::budget_tests` (deque-timeout shape, degenerate deferral, shim). Full suite: 1343 passed, 0 failed.

## Notes

- **Backstop (open question — decision pending):** the optional staleness backstop (`now > batch_start_epoch + k·profile_timeout`, `k = 4`, → `BatchIncomplete (reason: backstop)`) is **not implemented** — per the phase instruction, it is flagged *needs your call* in the plan and must not be built until decided. The stamps it would consume (`batch-start-epoch`, carried on the continuation file + both loom events) are in place, so adoption is a small dequeue-time check. Queue-wait exemption means a stale batch can span a large wall-clock span even though total *execution* ≤ `profile_timeout`, and `MAX_CONTINUATIONS` bounds hop count, not span.
- `execution_secs` is wall-clock (dequeue → handoff), total processing time including Knot overhead — locked decision; the runner-metadata variant (agent-invocation only) is a future refinement. Both satisfy the invariant `Σ execution_secs ≤ profile_timeout`.
- The budget only ever *decreases*; it is never reset to a full `profile_timeout` at any handoff ("fresh context, never a fresh budget").
- The compatibility shim is read-only (a `budget-secs` key always wins over a `batch-deadline-epoch` key when both are present); new continuations never write `batch-deadline-epoch`, so the shim can be removed after one release.
