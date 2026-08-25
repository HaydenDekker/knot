# Phase 1: Identity normalisation in the event store (TDD)

**Plan:** [Queue Entry Identity Self-Heal — Filename Is the Event ID](queue-identity-self-heal-plan.md)
**Commit:** `8b2fdd4` (branch `refactor/queue-identity-self-heal-plan`, stacked on 076's completed work)
**Date:** 2026-08-25

## Checklist
- [x] `scan_events()` normalises identity: filename stem is authoritative — on divergence the file is repaired in place via the existing atomic temp→rename `write_event` path with `id := stem`, one stderr warning per repaired file in the plan's exact format: `[queue] repaired event file {name}: id {old} -> {stem} (filename is the queue identity)`. `queued_at` and all other fields preserved; second scan performs no rewrite (idempotent). Files are collected before processing so repair renames don't race directory iteration
- [x] Missing `id` remains a parse failure (malformed skip); empty `id` and stemless `.json` filename are explicit malformed skips; the malformed-skip warning is extended with the name/id-rule hint
- [x] `front()`: unchanged shape (scan → read head); a `read_event` failure (file vanished between scan and read) now logs `[queue] head {name}.json vanished before read (concurrent removal?)` and returns `None` — never panic, never silent (replacing the swallow-with-`.ok()`)
- [x] `pop()`: the `.expect("failed to read event file on pop…")` is replaced with one rescan-and-retry; if the (re)scanned head is still unreadable or the queue drained concurrently, the vanished-head warning is logged and `None` is returned — graceful, no panic
- [x] `push_or_replace` / `load_persisted` / `delete` / `StrandQueueAccessor::delete` — untouched, no signature changes; correctness follows from the healed invariant, pinned by the new tests
- [x] 7 new unit tests, all green: `scan_repairs_renamed_file_and_normalises_id` (event_store.rs); `front_returns_renamed_head_without_wedging`, `pop_reads_renamed_head_without_panicking`, `front_vanished_head_is_graceful_none`, `pop_vanished_head_rescans_and_returns_tail`, `dedup_removes_renamed_file_by_healed_id`, `late_removal_deletes_the_peeked_file` (disk_event_queue.rs — the last pins the exact incident-#2 orphan path)
- [x] `cargo build` clean; `cargo test --no-fail-fast` green — 875 lib tests (868 + 7) + all integration binaries; `late_removal`, `persistent_queue`, `step` unchanged in expectation; plan 076's work on this branch untouched

## TDD red phase
5 of 7 tests failed against the current code, exactly as the incident mechanism predicts: `front()` → `None` wedge on a renamed head, `pop()` panic via `.expect`, scan not normalising, dedup leaving a dangling duplicate, and the late-removal orphan path. The 2 that passed pre-fix (`front_vanished_head_is_graceful_none`, `pop_vanished_head_rescans_and_returns_tail`) pin behaviour the old code happened to already satisfy (a hand-deleted head is simply absent from the scan; the scan-then-vanish window isn't forceable from a test).

## Deviations
1. **Vanished-head warning names the file with its extension** — `head {id}.json` rather than bare `{id}`, consistent with the repair warning's use of the full filename. The plan's `{name}` placeholder is satisfied by the full filename.
2. **Orchestrator verification flake (environment, not code).** The orchestrator's post-phase full-suite run aborted: `tests/thinking_level` exited non-zero with **zero output** (binary died before printing anything — not an assertion failure; no OOM in system logs, 61 GB RAM available). Follow-up runs: 4 consecutive full-suite greens (1201 tests, all 28 binaries ok), including `thinking_level` in-suite (2 passed, 0.05 s). Two of ~six recent full-suite runs flaked under load — consistent with the shared-GPU/machine contention (the thinking-level mock-CLI spawn sits inside a 10 s runner timeout). No test code changed; recorded here so a repeat at the Phase 2/3 gates isn't mistaken for a regression.

## Discoveries
- The repair rewrite reuses `write_event` (temp→rename), so the healed file lands at the *operator's* filename — the backdate/rename keeps its FIFO position and only the JSON `id` converges to the stem.
- `scan_events` now returns events whose `id.0` always equals the on-disk filename stem, which is what makes every downstream id-based operation (read, remove, dedup, snapshot) correct with no signature changes.
- The vanished-head path is the one place a `None` from `front()` is *expected* (concurrent late-removal / `knot step` between scan and read) — it is now visible in the service log, closing the "silent None" gap from the incident.

## Notes
- Phase 2 (incident-reproduction integration tests in `tests/queue_identity.rs`) is next.
