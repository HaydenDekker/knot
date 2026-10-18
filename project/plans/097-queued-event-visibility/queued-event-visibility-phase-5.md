# Phase 5: end-to-end integration test (mock runner)

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 4 is green and committed.**

## Checklist
- [x] A full-pipeline test in `tests/` alongside `pipeline.rs` /
      `persistent_queue.rs`: producer emits → entry lands in the queue →
      producer re-enters (same strand, `Modified`) → the **queued entry**
      (not the directory file) appears in the prompt with the norm; the mock
      runner shows the re-entry emitting `event: None` when the queued event
      covers the same facts (per the current convention — an ack block with
      `occurred: false`; see Deviations).
- [x] Assert the cap end-to-end: seed a backlog beyond `PENDING_INDEX_CAP`
      (15 queued entries) and assert the prompt shows the count + the newest
      10 — not all 15.
- [x] Assert omit-when-empty end-to-end: a queue holding only entries that
      must not surface (a type the knot cannot emit; a same-type entry from
      another producer) → the re-entry prompt carries no `## Queued Events`
      section.
- [x] Verification — `cargo clippy --all-targets` clean (0 errors; the new
      test file generates zero warnings — the 14 warnings attributed to the
      `queued_events_e2e` target are all pre-existing `tests/helpers.rs`
      lints, identical to the `pipeline` target's count); the new integration
      test is green (`cargo test --test queued_events_e2e`: 10 passed — 3
      plan-097 tests + 7 `helpers`-module tests); `cargo test --test pipeline`
      still green (19 passed); commit sha `⟨TBD⟩` recorded as this phase's
      external-verdict request. The ON-COMMIT suite is not required by this
      item.

## Deviations
- **`event: None` → `occurred: false` ack (convention change since the plan):**
  the plan's mock-runner sketch says "the mock runner shows the re-entry
  emitting `event: None` when the queued event covers the same facts". The
  event-block convention has since moved to `occurred: true|false`
  (post-0.43.0, per the tie-off content section and the 096-series docs):
  an un-emitted event is acknowledged with an `occurred: false` block and a
  *why* — not a bare `event: None`. The test asserts the current convention
  (the re-entry tie-off carries `event: PlanCreated` + `occurred: false` + a
  description referencing the queued event) and that such an ack is
  enforcement-satisfying (`KnotEventsMissing` absent) and **never
  dispatched** (exactly one `EventsDispatched` in the whole run).
- **The queue entry is pushed by the test, not by the watcher:** in
  production the debounced filesystem watcher moves dispatched entries into
  the queue. The integration test drives the queue directly
  (`InMemoryEventQueue::push` with the same `PendingEvent` shape) — the
  watcher itself is exercised by the plan-096 acceptance tests; this pane
  asserts the *consumer side* (prompt assembly from the queue) end-to-end.
- **Real dispatcher, mock runner:** the producer's session-1 dispatch uses
  the real `FileSystemEventDispatcher` (rig dir under a tempdir) so the
  queued event file exists on disk with the real frontmatter
  (`target-knot: plan-creator` — the producer identity the queued index
  reads). The agent itself stays a mock runner (per AGENTS.md: no live rig
  runs).

## Discoveries
- **Fixture pattern:** the pane reuses the plan-071
  `ProcessStrandBuilder` (`tests/helpers.rs`) — `with_looms` +
  `with_real_event_dispatcher` + `with_strand_queue` — so the test asserts
  the prompt of a *full* `ProcessStrand::execute` (config resolution,
  context assembly, enforcement), not a hand-built prompt. The producer's
  emission instructions (from the consumer loom's `PlanCreated`
  subscription) are what make the knot "may emit PlanCreated" — without the
  consumer loom the section is empty even with a full queue (no emittable
  types), which is itself the correct behaviour.
- **Queue path + queue timestamp are what distinguish the queue-backed
  view from the directory view:** the re-entry prompt assertion checks for
  the dispatched file's path *and* the queue's own `queued_at`
  (`queued: 2026-10-18T10:00:00`) — a directory-scan fallback would not show
  a `queued:` timestamp, so the assertion pins the queue path as the source.

## Notes
- `tests/queued_events_e2e.rs` — three tests:
  `queued_entry_appears_in_reentry_prompt_and_ack_dispatches_nothing`,
  `backlog_beyond_cap_shows_count_and_newest_cap_entries`,
  `no_queued_entry_of_emittable_types_omits_the_section`.
