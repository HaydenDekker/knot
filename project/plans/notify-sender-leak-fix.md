# Plan: Notify Sender Leak Fix — Immediate Cascade Drain

## Problem

The notify background thread holds an `Arc` reference to `InnerState`, which contains the `mpsc::Sender` instances for strand and config event channels. When `NotifyEventSource` is dropped during server shutdown, the notify thread's `Arc` keeps the senders alive, preventing channel closure and blocking the cooperative cascade drain.

This was mitigated in plan 21 with a 5-second timeout safety net (`tokio::time::timeout` + `join_set.abort_all()`). The timeout works but is a band-aid — it means every shutdown waits 5 seconds before aborting tasks, even when the cascade should drain in milliseconds.

### Root Cause

`InnerState` bundles senders with watch metadata:

```
InnerState {
    strand_sender: Sender<StrandEvent>,    ← lives in Arc
    config_sender: Sender<ConfigEvent>,    ← lives in Arc
    watched_dirs: Vec<...>,                ← lives in Arc (callback needs this)
    project_root: PathBuf,                 ← lives in Arc (callback needs this)
}
```

The notify callback captures `Arc<Mutex<InnerState>>`. When `NotifyEventSource` drops, the notify thread's `Arc` keeps `InnerState` alive → senders never drop → `recv() → None` never fires.

## Target

Split `InnerState` so senders live in the `NotifyEventSource` struct (not the `Arc`), and the notify callback uses `Weak` references to the senders. On `NotifyEventSource` drop, senders drop immediately → channels close → cascade drains in milliseconds.

### New structure

```
NotifyEventSource {
    strand_sender: Arc<Mutex<Sender<StrandEvent>>>,
    config_sender: Arc<Mutex<Sender<ConfigEvent>>>,
    watched_state: Arc<Mutex<WatchedState>>,     ← callback captures this Arc
}

WatchedState {
    watched_dirs: Vec<(PathBuf, WatchType)>,
    project_root: PathBuf,
}
```

The callback closure captures `Arc<Mutex<WatchedState>>` and receives `Weak` references to the senders. `Weak::upgrade()` succeeds during normal operation, fails silently after drop.

### ADR references

- **[ADR-001](../adrs/adr-001-integration-test-server-pattern.md)** — test pattern: `tokio::spawn` without graceful shutdown. Tests already use `spawn_server`, so no test changes needed.
- **[ADR-002](../adrs/adr-002-server-child-tasks.md)** — server child tasks: cooperative cascade shutdown. This plan eliminates the need for the 5-second timeout in `start_server_with_shutdown`.
- **[ADR-003](../adrs/adr-003-channel-cascade-shutdown.md)** — pattern 1: "no leaked senders" is the ideal case. This plan implements it by separating senders from the notify callback's `Arc`.

### Hex layer

**Outbound Adapters** — `NotifyEventSource` is an adapter. The port trait (`EventSource`) is unchanged. Domain and application layers are unaffected.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `event_source::tests` (16 tests) | Watcher starts, strand events, config events, directory filtering, path mapping | ✅ Green — all pass, use `create_source`/`create_source_fresh` helpers |
| `tests/task_management.rs` (5 tests) | Pipeline drain, debounce flush, in-flight completion | ✅ Green — validates cascade shutdown |
| `tests/shutdown.rs` (2 tests) | Graceful shutdown, watcher stop | ✅ Green — validates LoomStopped logging |
| `tests/generic_task_management.rs` (10 tests) | Generic cascade pattern (zero domain types) | ✅ Green — validates channel closure |

## Test Gaps

- No test for "drop `NotifyEventSource` → senders drop → channels close" — this is the exact property being fixed
- No test for `Weak` reference invalidation after drop
- No test for "notify callback event after drop → silent no-op"

## Phases

### Phase 0: Domain — No Changes

**Hex layer:** Domain (no changes needed)

The domain layer has no involvement in this refactor. `NotifyEventSource` is an outbound adapter — the port trait (`EventSource`) interface is unchanged, and domain types (`StrandEvent`, `ConfigEvent`) are unchanged.

- [ ] Verify: no domain imports needed for this change

### Phase 1: Outbound Adapter — Split State, Weak Senders

**Hex layer:** Outbound Adapters (`event_source.rs`)

- [ ] Split `InnerState` into:
  - `WatchedState` — `watched_dirs` + `project_root` (lives in `Arc`, captured by callback)
  - Senders stay in `NotifyEventSource` struct via `Arc<Mutex<Sender>>`
- [ ] Refactor `NotifyEventSource` constructor:
  - Create `WatchedState` in `Arc`, clone `Weak` for callback
  - Create senders in `Arc<Mutex<>>`, store in struct fields
  - Pass `Arc<WatchedState>` + `Weak<Sender>` clones to callback closure
- [ ] Refactor callback closure:
  - Capture `Arc<Mutex<WatchedState>>` (for event mapping)
  - Use `strand_sender_weak.upgrade().and_then(|s| s.try_send(event))` for sending
  - Same pattern for `config_sender_weak`
- [ ] Refactor `map_event`, `map_strand_event`, `map_rig_event`, `map_loom_event`:
  - Move from `impl InnerState` to `impl WatchedState` (methods are pure — no sender access)
  - Add `project_root` parameter where needed (currently accessed via `self`)
- [ ] Refactor `register_watch`, `watch`, `unwatch`, `set_loom_ids`:
  - Access senders via `self.strand_sender`, `self.config_sender` (struct fields, not Arc state)
  - Access watched dirs via `self.watched_state`
- [ ] Update `EventSource` trait impl
- [ ] Update `with_loom_ids`, `with_ids`, `register_watch` inherent methods
- [ ] Update test helpers: `create_source`, `create_source_fresh`
- [ ] Verify: all 16 existing `event_source::tests` pass

### Phase 2: Server — Remove Timeout Safety Net

**Hex layer:** Composition Root (`server.rs`)

- [ ] Revert the 5-second timeout in `start_server_with_shutdown`:
  - Replace `tokio::time::timeout(drain_timeout, async { ... })` with `while let Some` loop
  - Remove `join_set.abort_all()` fallback
  - Restore original cooperative drain loop
- [ ] Update comments to reference ADR-003 pattern 1 (no leaked senders) instead of pattern 4 (timeout)
- [ ] Verify: `cargo build` succeeds

### Phase 3: Integration Tests — Verify Cascade Drain

**Hex layer:** Integration Tests

- [ ] Add new test to `event_source::tests`: `drop_source_channels_close`:
  - Create `NotifyEventSource`, watch a directory, drop source
  - Verify `recv()` on strand/config channels returns `None`
  - Proves senders dropped immediately on struct drop
- [ ] Add new test: `notify_callback_silent_after_drop`:
  - Create source, watch directory, drop source
  - Write file to trigger notify callback
  - Verify no event sent to channels (callback uses `Weak`, upgrade fails)
- [ ] Run `tests/task_management.rs` — verify cascade drain works without timeout
- [ ] Run `tests/shutdown.rs` — verify LoomStopped written promptly
- [ ] Run `tests/generic_task_management.rs` — verify generic pattern unchanged
- [ ] Verify: full test suite passes (196 lib + 82 integration)

### Phase 4: Documentation — Update ADR References

**Hex layer:** Documentation

- [ ] Update [ADR-002](../adrs/adr-002-server-child-tasks.md):
  - Add note: "Notify sender leak fixed by separating senders from callback Arc (plan 22)"
  - Update cascade diagram to show immediate drain (no timeout)
- [ ] Update [ADR-003](../adrs/adr-003-channel-cascade-shutdown.md):
  - Add `NotifyEventSource` as concrete example of pattern 1 (no leaked senders)
  - Keep pattern 4 (timeout) as fallback for other scenarios
- [ ] Update domain glossary if needed (unlikely)

## Notes

- The notify thread itself continues running after drop — this is fundamental to how the notify crate works (separate `std::thread`). The fix only ensures the *senders* drop, not the thread. Thread cleanup is handled by the notify crate's own lifecycle.
- `Weak::upgrade()` on every event adds minimal overhead (one atomic load + compare). This is acceptable because notify events are low-frequency (a few hundred/day).
- The 5-second timeout from plan 21 is removed from production code. It was a safety net for the leaked sender — with the leak fixed, it's no longer needed.
