# Phase 4: Observability + documentation

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [x] Extend `LoomEvent::EventsDispatched.dispatches` to `Vec<(String, String, String, String)>` (4th = created file path, **absolute** — the path the dispatcher returned); doc comment notes the 0.33.0 addition and the legacy-skip behaviour (0.30.1 precedent)
- [x] Carry the `dispatch()` return path through `dispatch_events_to_consumers` (`path.display().to_string()` into the 4th tuple element) into the `EventsDispatched` entry; return type updated to the 4-tuple
- [x] Update all `EventsDispatched` consumers/tests — no breakage (tests used index access/`matches!`); extended `event_dispatch_full_flow` to assert the 4th element carries the dispatcher-created path (mock: ends `/consumer-loom/PlanCreated/event-mock.md`)
- [x] Loom-log test: `loom_log_read_all_skips_legacy_3tuple_events_dispatched` — hand-written log with a legacy 3-tuple `EventsDispatched` line between two current-shape lines; the legacy line is skipped (warning path) while both current lines read in order; also asserts the 4th element round-trips
- [x] Update doc comments: `event_dispatcher.rs` module docs gain a "Filename contract" section (`event-{ts}[-NNN].md`, atomic creation, transparent to consumers) and the struct doc notes atomicity; `ports.rs` port docs done in Phase 1 (seq contract + adapter-owned materialisation); `events.rs` event docs updated in this phase
- [x] `knot-update` changelog entry: "Unique Event Dispatch Filenames — Per-Batch Sequence Suffix (Knot 0.33.0, 2026-08-22)" — incident recap, naming table, atomicity, `EventsDispatched` tuple expansion (internal runtime artifact, no migration, warnings self-settle); frontmatter bumped to `version 1.9.0` / `compatibility Knot 0.33.0+`
- [x] Record separator decision — **hyphen** (`event-{ts}-001.md`), per the plan's preference: consistent with the existing hyphen-heavy filename and keeps one parseable shape `event-<anything>[-NNN].md`
- [x] Compile and verify no errors
- [x] Run full test suite — 1077 passed across all targets (1 pre-existing lib failure)

## Deviations
- None.

## Discoveries
- The `EventsDispatched` expansion needed zero call-site changes: every consumer either pattern-matches with `..` or indexes tuple elements — only the construction site and the doc/tests were touched.
- The glossary (`knot-init/knot-glossary.md`) and other skills refer to event files generically (`<event-file>.md`, `event-*.md` examples) — no sync needed; the `knot-update` changelog is the canonical record of the shape change, as the plan scoped it.

## Notes
- Absolute path chosen for the 4th element (plan: "prefer absolute to match other path-carrying events") — it is the exact `PathBuf` the dispatcher returned, so the loom-log entry points at the real file on disk.
