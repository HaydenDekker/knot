# Phase 1: Queue-backed pending set (domain/application)

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 0 is green and committed.**

## Checklist
- [x] Add the cap constant `PENDING_INDEX_CAP: usize = 10` (domain — alongside
      the pending-set types). — `src/domain/pending_event.rs`.
- [x] Extend `StrandQueueAccessor` (`src/domain/events.rs`) with an entries view
      — `pending_entries() -> Vec<QueueEntryView { strand_path, queued_at }>` —
      so the index gets `queued_at` without file reads; default impl derives
      from `pending_strand_paths()` (empty `queued_at`) so existing test doubles
      keep compiling; implemented on `DiskBackedEventQueue`
      (`src/adapters/outbound/disk_event_queue.rs`, via `scan_events()`) and
      `InMemoryEventQueue` (`src/application/in_memory_event_queue.rs`).
- [x] Add the domain pending-set builder — `build_pending_index(metas,
      producer_id, emittable_types) -> Vec<QueuedEventType>` in
      `src/domain/pending_event.rs`, returning per-type `(count,
      newest-N {path, queued_at})`, ordered newest-first by `queued_at`. Event
      type derived from the dispatch path (parent directory name of the
      `event-*` file) — no file I/O at the domain level.
- [x] Apply the Phase 0 producer-identity decision: frontmatter-only (default) —
      the domain builder stays read-free; the `producer` arrives as
      frontmatter data the caller extracted (`QueuedEventMeta.producer`).
      Recorded in Deviations.
- [x] Write unit tests for the builder — the phase's pane:
      - cap: 50 queued entries of one type → count 50, listed entries 10, the
        10 newest by `queued_at`;
      - scoping: entries of non-emittable types excluded entirely;
      - empty: nothing of an emittable type → empty index (section omission is
        Phase 2's job);
      - metadata-only: the index builds from entry metadata + path alone — no
        `description`/`timestamp` carried, no body reads;
      - ordering: newest-first within each type, types stable.
      (+ producer-scoping and `count_event_types` helpers.)
- [x] Verification — `cargo clippy --all-targets` clean; `cargo test --lib
      pending` (the targeted builder + queue-accessor tests) green; commit
      sha `520412a` recorded as this phase's external-verdict request. The
      ON-COMMIT suite is not required by this item.

## Deviations
- Frontmatter-only producer identity (Phase 0 default D1) confirmed: the
  domain builder stays read-free; the `producer` field on
  `QueuedEventMeta` is frontmatter data the caller extracted. No queue-entry
  producer field was added (avoids a schema + `state.json` + adapter churn
  for a one-key read).
- The accessor method is `pending_entries()` (returning
  `Vec<QueueEntryView>`) rather than an inline tuple, so the type is named
  and reusable; the default implementation keeps the three existing
  `StrandQueueAccessor` implementors (`DiskBackedEventQueue`,
  `InMemoryEventQueue`, `OrderingQueue`) compiling without churn.

## Discoveries
- Two queue adapters needed the override (both already carry `queued_at`
  via `PendingEvent`); `OrderingQueue` (test) relies on the default.
- No `run-scope.md` / `scripts/test-data/` in this repo — verification uses
  the targeted `cargo test --lib` filters as the ALWAYS-equivalent.

## Notes
- Hexagonal order: the accessor trait lives in domain (it does today); the
  builder is pure domain logic; adapters implement the trait.
