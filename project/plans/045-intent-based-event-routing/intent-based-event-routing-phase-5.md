# Phase 5: Integration — Wire into processing pipeline

**Plan:** [Intent-Based Event Routing](intent-based-event-routing-plan.md)

## Checklist
- [x] Add `event_dispatcher: Arc<dyn EventDispatcherPort>` field to `ProcessStrand` struct — done, added to struct and constructor
- [x] Update `ProcessStrand::new()` to accept event dispatcher — done, added parameter and updated all callers (lib + integration tests + server.rs)
- [x] Add `EventsDispatched` variant to `LoomEvent` enum — done, added with `loom_id`, `knot_id`, `strand_path`, `dispatches: Vec<(String, String)>`, `timestamp` fields. Updated `loom_log.rs` match arm.
- [x] Inject listener context into prompt (per-invocation, not cached) — done:
  - `collect_all_knots()` helper collects all knots from all looms in the store
  - `build_listener_context(knot, &all_knots)` called per-invocation in `execute()` before building prompt
  - Context prepended to prompt for both normal and Deleted events
  - Uses live store snapshot — no caching
- [x] After successful knot completion (KnotCompleted path):
  - `dispatch_agent_events()` helper: parses tie-off for agent events via `extract_agent_events()`
  - For each event, finds all consumer knots with matching intents via `matches_intent()`
  - Dispatches event files to each consumer's loom tie-off directory via `EventDispatcherPort`
  - Logs dispatch to loom-log via `EventsDispatched` event
  - Best-effort — dispatch failures are non-fatal (uses `let _ = `)
- [x] Update test shared module (`build_process_strand`) to include mock event dispatcher — done, added `MockEventDispatcher::default()` to all test construction sites
- [x] Integration test: full flow — target runs → event detected → consumer event file created → loom-log entry — `event_dispatch_full_flow`
- [x] Integration test: no events — successful run with no events produces no dispatch — `no_events_no_dispatch`
- [x] Integration test: fan-out — one event matches consumers in two different looms — `event_dispatch_fan_out_two_looms`
- [x] Integration test: listener context injected when consumers exist — `listener_context_injected_in_prompt`
- [x] Integration test: no context injected when no listeners — `no_listeners_no_context_injection`
- [x] Compile and verify no errors — clean build with warnings only
- [x] Run full test suite — 673 tests pass, 0 failures

## Deviations
- Added two extra tests beyond the plan: `listener_context_injected_in_prompt` and `no_listeners_no_context_injection` to verify the prompt injection behaviour (which is part of Phase 5's integration but not explicitly listed in the plan checklist).

## Discoveries
- `build_listener_context` is a pure function (linear scan + HashSet dedup) — confirmed no I/O, making per-invocation calls cheap.
- The `dispatch_agent_events` helper needed to iterate by loom (not flat knots) to track `consumer_loom_id` for the dispatch call.
- The `LoomEvent` enum's pattern match in `loom_log.rs` needed updating for the new variant.

## Notes
- Event dispatch is best-effort in the KnotCompleted path — failures are logged but don't fail the strand processing.
- The `EventsDispatched` loom-log entry records `(event-id, consumer-loom-id)` pairs for traceability.
- All integration test construction sites (7 integration test files + lib test modules) were updated to include the `MockEventDispatcher`.
