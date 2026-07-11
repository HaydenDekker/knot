# Plan: Loom-Level Event Subscriptions

## Implementation Status: ✅ Complete (2026-07-11)

## Notes
- All 3 phases (0, 1, 2) implemented and verified
- Full test suite passes (625 lib tests + 100+ integration tests, 0 failures)
- Version bumped to 0.26.0
- Domain glossary `StrandSource` entry updated with loom-level subscription format

## Problem

Events are currently first-class citizens with knot-level subscriptions: a consumer declares `strand-dir: "event:<producer-knot>:<EventId>"` and Knot matches events by producer knot ID + event ID. This works for point-to-point agent communication.

However, there's no way to subscribe to events from an **entire loom**. If a loom has multiple knots that can all emit the same event type (e.g. `PlanCreated`), a consumer must declare a separate subscription per knot. This is repetitive and fragile — adding a new knot to the loom means updating all consumers.

Additionally, only the specific producer knot named in the subscription receives event injection in its prompt. With loom-level subscriptions, **every knot in the loom** should receive the event injection so it knows it must declare an event in its tie-off if the event occurred during its turn.

## Target

Consumer knots can subscribe to events from an entire loom using the format:

```yaml
strand-dir: "event:planning-loom:PlanCreated"
```

This means: "listen for `PlanCreated` events emitted by **any knot** within `planning-loom`."

Key behaviours:

1. **StrandSource parsing** — `event:<target>:<EventId>` resolves as loom-level if `<target>` ends with `-loom`, otherwise knot-level.
2. **Listener context injection** — when building the prompt for a knot, Knot checks if any consumer has a loom-level subscription for the knot's loom. If so, event instructions are injected (same format as knot-level). This means **every knot in the subscribed-to loom** receives the event injection.
3. **Event dispatch matching** — after a knot completes, Knot matches events against both knot-level subscriptions (existing: `producer_knot == knot.id`) and loom-level subscriptions (new: `target == knot.loom_id`). Events matching either are dispatched to the consumer's dispatch directory.
4. **Existing knot-level subscriptions** (`event:<knot-name>:<EventId>`) continue to work unchanged.

The `StrandSource::EventUri` variant stores the raw target string. Resolution to knot-level vs. loom-level happens at match time using helper methods that accept the full knot/loom registry. This avoids needing registry access at parse time.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `value_objects.rs::StrandSource` tests (12 tests) | Parsing plain paths, event URIs, malformed URIs, whitespace handling, path-with-colon edge case | ✅ Green — tests knot-level event URI parsing |
| `events.rs::build_listener_context_*` tests (10+ tests) | Context injection prompt generation, deduplication, event descriptions, multi-producer, markdown fence format | ✅ Green — tests knot-level context injection |
| `events.rs::AgentEvent` tests (7 tests) | Construction, serialisation, empty payload, body field | ✅ Green |
| `process_strand.rs` event dispatch tests | Event parsing, matching, dispatch fan-out | ✅ Green — tests knot-level dispatch |
| `config_event_handler.rs` tests | Knot added/modified/deleted, strand source watch setup | ✅ Green |
| `event_dispatcher.rs` tests (8 tests) | File dispatch, path creation, file content, fan-out | ✅ Green |

All existing tests validate the **current** knot-level subscription model. They must continue to pass after the change.

## Test Gaps

- No test for `StrandSource` parsing/identification of loom-level URIs (`event:planning-loom:PlanCreated`)
- No test for `build_listener_context` injecting events based on loom-level subscriptions
- No test for loom-level event dispatch matching
- No test for mixed loom-level + knot-level subscriptions to the same event
- No test for resolution ambiguity (target matches both a knot name and a loom name)

## Phases

### Phase 0: Domain — StrandSource resolution methods and tests

Add helper methods to `StrandSource` in `value_objects.rs` to determine whether an `EventUri` target refers to a knot or a loom:

- `is_loom_target(&self) -> bool` — returns `true` if the target string ends with `-loom` (the loom naming convention). Heuristic-based, no registry needed.
- `resolve_for_producer(&self, producer_knot_id: &str, producer_loom_id: &str, all_knot_ids: &[&str]) -> Option<EventSubscription>` — resolves the target against a known producer. Returns `Some(EventSubscription::KnotLevel)` if the target matches `producer_knot_id`, `Some(EventSubscription::LoomLevel)` if the target matches `producer_loom_id`, or `Some(EventSubscription::KnotLevel)` if the target matches any known knot name. Returns `None` if no match.
- New enum `EventSubscription` with variants `KnotLevel { producer_knot: String, event_id: String }` and `LoomLevel { producer_loom: String, event_id: String }`.

This is a pure domain-layer change. The `EventUri` struct fields remain unchanged (`producer_knot`, `event_id`) — the `producer_knot` field stores the raw target string (which may be a loom name). The resolution methods interpret it correctly.

**Tests (in `value_objects.rs`):**
- `strand_source_from_str_loom_uri` — parses `event:planning-loom:PlanCreated` as `EventUri`
- `strand_source_is_loom_target_true_for_loom_suffix` — target ending in `-loom` returns `true`
- `strand_source_is_loom_target_false_for_knot_name` — target not ending in `-loom` returns `false`
- `strand_source_resolve_for_producer_knot_match` — target matches producer knot ID → `KnotLevel`
- `strand_source_resolve_for_producer_loom_match` — target matches producer loom ID → `LoomLevel`
- `strand_source_resolve_for_producer_no_match` — target matches neither → `None`
- `strand_source_resolve_loom_suffix_takes_precedence` — target ending in `-loom` is loom-level even if a knot with that name exists
- All existing `StrandSource` tests continue to pass

### Phase 1: Context injection — loom-level subscriptions in `build_listener_context`

Update `build_listener_context` in `events.rs` to check for loom-level subscriptions in addition to knot-level subscriptions.

The function signature changes to accept the producer's loom ID:

```rust
pub fn build_listener_context(knot: &Knot, loom_id: &LoomId, all_knots: &[Knot]) -> String
```

Matching logic (per consumer knot):
1. For each consumer with `StrandSource::EventUri { producer_knot, event_id }`:
   - **Knot-level**: if `producer_knot == knot.id.0` → match (existing behaviour)
   - **Loom-level**: if `producer_knot.ends_with("-loom")` AND `producer_knot == loom_id.0` → match (new behaviour)
2. Group by event ID (deduplicate)
3. Build the event instruction list (same format as today)

When a loom-level subscription matches, **every knot in that loom** gets the event instructions injected — because `build_listener_context` is called per-knot during `ProcessStrand::execute`, and the loom ID is the same for all knots in the loom.

Update the caller in `process_strand.rs` to pass the loom ID.

**Tests (in `events.rs`):**
- `build_listener_context_loom_level_consumer_matches_producer_in_loom` — consumer with `event:planning-loom:PlanCreated` matches a knot inside `planning-loom`
- `build_listener_context_loom_level_consumer_no_match_different_loom` — consumer with `event:planning-loom:PlanCreated` does NOT match a knot inside `review-loom`
- `build_listener_context_mixed_knot_and_loom_consumers` — both knot-level and loom-level consumers for the same event appear (deduplicated by event ID)
- `build_listener_context_all_knots_in_loom_get_injection` — verify that every knot in the subscribed-to loom receives event instructions
- All existing `build_listener_context` tests continue to pass

### Phase 2: Event dispatch — loom-level matching in `dispatch_agent_events`

Update `dispatch_agent_events` in `process_strand.rs` to match events against both knot-level and loom-level subscriptions.

Current matching logic (in the consumer loop):

```rust
if producer_knot == &knot.id.0 && event_id == &event.event_id {
    dispatch(event, consumer_knot, &knot.id.0, &loom.id);
}
```

Updated matching logic:

```rust
let resolved = consumer_source.resolve_for_producer(
    &knot.id.0, &producer_loom_id, &all_knot_ids,
);
if let Some(sub) = resolved {
    match sub {
        EventSubscription::KnotLevel { event_id: sub_event_id, .. } => {
            if sub_event_id == event.event_id {
                dispatch(...);
            }
        }
        EventSubscription::LoomLevel { event_id: sub_event_id, .. } => {
            if sub_event_id == event.event_id {
                dispatch(...);
            }
        }
    }
}
```

The dispatch call is unchanged — it writes to `rig/tie-offs/{consumer-loom-id}/{event-id}/` regardless of whether the subscription is knot-level or loom-level.

**Tests (in `process_strand.rs` integration tests):**
- `dispatch_agent_events_loom_level_subscription_matches` — producer in subscribed loom dispatches to consumer
- `dispatch_agent_events_loom_level_subscription_no_match_different_loom` — producer in different loom does not dispatch
- `dispatch_agent_events_mixed_knot_and_loom_subscriptions` — both types of consumers receive the event
- All existing event dispatch tests continue to pass

## Notes

- The `-loom` suffix heuristic for distinguishing loom-level from knot-level URIs is consistent with the loom naming convention already used throughout Knot (looms are directories ending in `-loom`). If a knot happens to be named with a `-loom` suffix, the loom-level match takes precedence. This is a reasonable constraint — knot names should not end with `-loom`.
- The `producer_knot` field in `StrandSource::EventUri` stores the raw target string regardless of whether it's a knot name or loom name. This is a minor semantic mismatch in the field name but avoids breaking changes to the struct layout. The field name could be renamed to `target` in a future refactor.
- The `EventSubscription` enum is a domain value object that captures the resolved subscription type. It is used only during matching — not persisted or serialized.
- Backward compatibility: all existing `event:<knot-name>:<EventId>` URIs continue to work because they don't end with `-loom` and resolve as knot-level subscriptions.
- The domain glossary `StrandSource` entry needs updating to document the loom-level subscription format.
