# Phase 1: Queue-backed pending set (domain/application)

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 0 is green and committed.**

## Checklist
- [ ] Add the cap constant `PENDING_INDEX_CAP: usize = 10` (domain — alongside
      the pending-set types).
- [ ] Extend `StrandQueueAccessor` (`src/domain/events.rs`) with an entries view
      — e.g. `pending_entries() -> Vec<QueueEntryView { strand_path, queued_at }>`
      — so the index gets `queued_at` without file reads; implement it on
      `DiskBackedEventQueue` (`src/adapters/outbound/disk_event_queue.rs`, via
      `snapshot()`) and on every test double implementing the trait.
- [ ] Add the domain pending-set builder — a function (e.g. in
      `src/domain/pending_event.rs` or a new `src/domain/queued_event_index.rs`)
      taking (queue entries, the knot's emittable event types, the cap) and
      returning per-type `(count, newest-N {path, queued_at})`, ordered
      newest-first by `queued_at`. Event type derived from the dispatch path
      (parent directory name of the strand path) — no file I/O at the domain
      level.
- [ ] Apply the Phase 0 producer-identity decision: if frontmatter-only
      (default), the domain builder stays read-free and the provider (Phase 2)
      does the `target-knot` frontmatter filter; if a queue-entry producer field
      was chosen, add the field (additive serde default) and filter in the
      builder. Record which in Deviations.
- [ ] Write unit tests for the builder — the phase's pane:
      - cap: 50 queued entries of one type → count 50, listed entries 10, the
        10 newest by `queued_at`;
      - scoping: entries of non-emittable types excluded entirely;
      - empty: nothing of an emittable type → empty index (section omission is
        Phase 2's job);
      - metadata-only: the index builds from entry metadata + path alone — no
        `description`/`timestamp` carried, no body reads;
      - ordering: newest-first within each type, types stable.
- [ ] Verification — `cargo clippy --all-targets` clean; `cargo test --lib
      pending` (the targeted builder + queue-accessor tests) green; commit
      sha `⟨TBD⟩` recorded as this phase's external-verdict request. The
      ON-COMMIT suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- Hexagonal order: the accessor trait lives in domain (it does today); the
  builder is pure domain logic; adapters implement the trait.
