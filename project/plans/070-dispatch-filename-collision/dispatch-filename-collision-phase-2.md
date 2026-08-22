# Phase 2: Atomic creation + taken-name fallback

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [ ] Replace `std::fs::write` in `FileSystemEventDispatcher::dispatch` with `OpenOptions::new().write(true).create_new(true)` resolve loop: on `AlreadyExists`, bump suffix (from `seq + 1`, or from 1 when `seq == 0`), retry up to cap (1000), `PortError` on exhaustion
- [ ] Write content to the opened handle and flush
- [ ] Deterministic test (injected timestamp via helper-level or refactored dispatch path): pre-create `event-{ts}.md` → `seq = 0` lands on `event-{ts}-001.md`
- [ ] Deterministic test: pre-create `event-{ts}-001.md` → `seq = 1` lands on `event-{ts}-002.md`
- [ ] Deterministic test: exhaustion cap returns a clear `PortError`
- [ ] Verify no existing test depends on overwrite semantics
- [ ] Compile and verify no errors
- [ ] Run full test suite (lib + integration)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
