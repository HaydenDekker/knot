# Phase 2: Wire the clear into `run_startup`

**Plan:** [Clear Loom-Logs and Rig-Log at Startup](startup-log-clear-plan.md)

## Checklist
- [ ] Write acceptance-level composition test `test_startup_clears_logs_before_discovery` in `src/server.rs` (real adapters, pre-populated runtime root incl. orphan loom + unparseable rig-log line)
- [ ] Insert `clear_all()` + `clear()` calls in `run_startup` after `migrate_legacy_rig_layout`, before `DiscoverLooms` (non-fatal `WARNING:` on error)
- [ ] Adjust `test_startup_migrates_legacy_layout` post-startup assertion (moved loom-log is cleared, not content-preserved)
- [ ] Run full suite (`cargo test`) — adapters, rig_log, migration_* composition tests stay green
- [ ] Decide open question: prune empty-only loom dirs? (plan default: no)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
