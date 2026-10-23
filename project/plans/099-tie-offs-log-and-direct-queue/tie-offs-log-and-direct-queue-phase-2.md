# Phase 2: Direct queueing from the tie-off parse

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 1 is green and committed** (Phase 1: sha
`0e0c501`).

## Checklist
- [x] `QueueEventDispatcher` adapter
      (`src/adapters/outbound/queue_event_dispatcher.rs`, new) —
      implements `EventDispatcherPort` as a direct queue write: one
      payload-bearing `PendingEvent` per subscriber; the returned
      delivery reference is the entry's `event:<producer>/<event-id>`
      descriptor (logged verbatim by the caller)
- [x] `EventDispatcherPort::dispatch` gains `original_strand` (the
      producer's own trigger strand; `None` for loom-/rig-scoped system
      events); `DispatchRequest` carries it; the port contract is
      reworded from file creation to delivery
- [x] The producer's loom is stamped into every emitted payload
      (`producer-loom`) — agent events in `dispatch_agent_events`,
      system events scope-aware in `SystemEventEmitter::emit` (knot/
      loom-scoped → the loom, rig-scoped → the rig id)
- [x] One persistent queue shared by both delivery paths
      (`AppContext.event_queue` in `src/server.rs`) — the system-event
      emitter and the strand processor deliver into and drain from the
      same `DiskBackedEventQueue`; the pipeline drains the shared queue
      instead of creating its own
- [x] The `event:`-URI strand/watch path retires
      (`ensure_event_uri_watch` in `mod_watchers.rs`) — no dispatch
      directory, no watcher, no `DirectoryCreated`; a `watch-skipped`
      diagnostic is logged; the subscription itself
      (`StrandSource::EventUri`) and producer-side matching
      (`resolve_for_producer`) are unchanged
- [x] The pending-event visibility index repoints to the entry-carried
      ids (Phase 0 decision D6) — `QueueEntryView` gains `event_id` /
      `producer` (populated from the delivery by both queue
      implementations); `QueuedEventMeta` gains `event_id`;
      `event_type_from_path` recognises the descriptor with the legacy
      path-derivation kept for transition;
      `AgentEventsContextProvider` prefers the carried values over the
      frontmatter read; the index stays metadata-only
- [x] The `FileSystemEventDispatcher` conformed to the new port
      signature (kept until Phase 5 retires the path with diagnostics)
- [x] Absence assertions in the e2e pane — `tests/step.rs` (the
      captured dispatch is an `Event`-kind entry with the descriptor and
      the delivery payload, not a debounced file watch);
      `tests/system_event_subscriptions.rs` (the static-token consumer
      runs from the queue entry — the prompt names the event and the
      actual rig id — and the per-`EventId` directory holds no `.md`);
      the config-handler test (an event-URI knot registers no watcher)
- [ ] Verification — clippy clean for the new code; the targeted panes
      green (`tests/step` 17 passed, `tests/system_event_subscriptions`
      13 passed, `tests/payload_queue` 21 passed — verified in-tree
      2026-10-23); the commit sha to be recorded when this phase lands.
      The ON-COMMIT suite is not required by this item.

## Deviations
- The port change landed wider than the file set implied: threading
  `original_strand` through `dispatch_grouped` touched the shared
  system-event path (plan 082) and every `DispatchRequest` site — the
  same delivery must carry the origin for system events too (D5).

## Discoveries
- The dispatcher sees only the producer token, not its loom — the loom
  now travels in the payload's `producer-loom` field, stamped by the
  emitter (which knows its scope); the dispatcher falls back to the
  producer token when the field is absent.
- The legacy-layout migration (`migrate_legacy_rig_layout`) must run
  **before** the events dir is created — creating the runtime root
  first would make a legacy `rig/tie-offs/` look like a conflict and
  never be moved. The queue now being created with the context (not in
  the pipeline) forced this ordering into `build_app_context`.

## Notes
- System events ride the same direct delivery (Phase 0 decision D5) —
  this phase is what retires the file mechanism entirely; a queue-only
  path for agent events would have kept the whole watcher dependency
  alive for no reason.
- Working tree at flesh-out time (2026-10-23): implementation complete,
  targeted panes green, uncommitted — the verification item lands with
  the commit.
