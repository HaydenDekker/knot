# Plan: Fix KnotModified race and GET knot-status hang

**Status:** Complete (2026-06-14)
**Branch:** (merged to main)

## Timeline

- 2026-06-08: Phases 0–3 complete, merged to main
- 2026-06-14: Phase 4 added (QueueIdle drain-check bug), completed inline

## Implementation Status: ✅ Complete (2026-06-14)

## Notes
- Phase 4: QueueIdle drain-check bug — `is_burst_active` flag replaces nested loop
- Debug `eprintln!` logging removed (kept only QueueIdle write confirmation)
- All 18 pipeline/rig_log tests pass
- Live-verified on borrow-my-stuff rig (burst of 2 events → QueueIdle written)
**Branch:** `fix/knot-modified-race-and-status-read`

---

## Problem

### Issue 1: `KnotModified` can't recover from a missed knot registration

When a loom directory is created on the filesystem with a knot file inside it, the notify watcher fires `LoomAdded` before the knot file is fully written. `handle_loom_added` scans the loom directory, the YAML parser fails (`InvalidFormat`), and the loom is registered with **0 knots**. When `KnotModified` fires moments later with the valid parsed knot, `handle_knot_modified` returns `LoomSaveFailed("knot not found")` — the knot is permanently lost.

This also affects HTTP-created looms: `POST /looms` writes the `.md` file then calls `RegisterLoom`. If the notify watcher fires `LoomAdded` between the file write and the in-memory registration, the same race occurs.

**Reproduction:** Create a `*-loom/` directory and drop a `.md` file into it while Knot is running. If the timing aligns, the loom appears with 0 knots.

### Issue 2: `GET /looms/{id}/knots/{name}` may block the HTTP handler

`GetKnotStatus::execute()` calls `log_port.read_all()` which does synchronous `fs::File::open()` + `BufReader::lines()` on the axum worker thread. Under contention (agent subprocess writing to `.loom-log` simultaneously), the read may block the handler. This is unlikely to be a deadlock — `append()` is short-lived — but a long loom-log file read on the tokio thread can delay all HTTP responses. The fix is to prove the behavior exists and add a safety net.

---

## Changes

### Phase 0: `KnotModified` recovery (unit tests + implementation)

**File:** `src/application/usecases.rs`

When `handle_knot_modified` finds the knot is not in the store, instead of returning an error, treat it as a new registration: append the knot to the loom, log `KnotRegistered`, and start a watcher. Log a warning so we can measure how often this path fires.

**Unit tests to add:**

| Test name | Assertion |
|-----------|-----------|
| `config_handler_knot_modified_new_knot_registers` | `KnotModified` for a loom with 0 knots → knot added, `KnotRegistered` logged, watcher started |
| `config_handler_knot_modified_warns_on_recovery` | Verify the warning log line is emitted (check log output via a capture mechanism or verify the code path exists) |

Update existing test:

| Existing test | Change |
|---------------|--------|
| `config_handler_knot_modified_not_found` | Change from "returns error" to "recovers by registering the knot" |

- [x] Modify `handle_knot_modified` to treat missing knot as new registration: append knot to loom, log `KnotRegistered`, start watcher, emit warning
- [x] Add unit test `config_handler_knot_modified_new_knot_registers`
- [x] Add unit test `config_handler_knot_modified_warns_on_recovery`
- [x] Update existing test `config_handler_knot_modified_not_found` to expect recovery
- [x] Compile and run tests

### Phase 1: HTTP `POST /looms` verifies auto-discovery (integration test)

**File:** `tests/auto_discovery_and_knot_crud.rs`

Add an integration test that:
1. Starts server with empty rig
2. Calls `POST /looms` to create a loom with 1 knot
3. Verifies the loom is registered with the correct knot count (not 0)
4. Verifies the `.loom-log` contains `KnotRegistered` for the knot

This proves the HTTP creation path is resilient to the notify race — either the HTTP handler registers directly, or auto-discovery pre-registers and the HTTP handler is idempotent.

- [x] Add integration test `http_post_loom_verifies_knot_registered` in `tests/auto_discovery_and_knot_crud.rs`
- [x] Test verifies loom registered with correct knot count after `POST /looms`
- [x] Test verifies `.loom-log` contains `KnotRegistered`
- [x] Compile and run tests

### Phase 2: Filesystem loom creation race (integration test)

**File:** `tests/auto_discovery_and_knot_crud.rs`

Add an integration test that reproduces the race:
1. Start server with empty rig
2. Create a `*-loom/` directory
3. **Immediately** write the `.md` file (minimising time between dir creation and file write)
4. Poll `GET /looms/{id}` until the loom has the expected knot count

This proves the `KnotModified` recovery path works end-to-end. If the test consistently passes, the fix is verified.

- [x] Add integration test `filesystem_loom_creation_race_recovery` in `tests/auto_discovery_and_knot_crud.rs`
- [x] Test creates loom directory then immediately writes `.md` file
- [x] Test polls `GET /looms/{id}` until expected knot count
- [x] Compile and run tests

### Phase 3: Prove and fix knot-status read blocking

**File:** `tests/pipeline.rs` (or new `tests/knot_status_concurrent.rs`)

Add an integration test:
1. Start server with a loom + knot using a **slow mock agent** (e.g., `sleep 10` before writing output)
2. Create a strand to start processing
3. While the agent is mid-processing (agent is running, loom-log has `KnotProcessing` but not `KnotCompleted`), concurrently send 10 `GET /looms/{id}/knots/{name}` requests
4. Assert all requests complete within a reasonable timeout (e.g., 5 seconds)

If any request hangs/times out, the issue is confirmed. The fix is to wrap `read_all()` in `tokio::task::spawn_blocking` inside the handler.

- [x] Add integration test `knot_status_during_processing_does_not_hang`
- [x] Test uses slow mock agent, sends 10 concurrent knot-status requests
- [x] Test asserts all requests complete within 5 seconds
- [x] Apply `spawn_blocking` fix in `src/adapters/inbound/loom.rs` (as safety net even if not reproducible)
- [x] Compile and run tests

**Handler fix** (if Phase 3 proves blocking):

In `src/adapters/inbound/loom.rs`, `get_knot_status`:

```rust
// Before (blocking read on axum worker thread):
let use_case = GetKnotStatusUc::new(ctx.store.clone(), Arc::clone(&ctx.loom_log_port));
match use_case.execute(&loom_id_val, &knot_id) { ... }

// After (spawn_blocking for filesystem read):
let use_case = GetKnotStatusUc::new(ctx.store.clone(), Arc::clone(&ctx.loom_log_port));
let result = tokio::task::spawn_blocking(move || {
    use_case.execute(&loom_id_val, &knot_id)
}).await;
match result {
    Ok(Ok(status)) => ...,
    Ok(Err(_)) => ...,
    Err(_) => ..., // task panicked
}
```

---

### Phase 4: QueueIdle drain-check bug — `QueueIdle` never written after last event

**File:** `src/server.rs`

#### Problem

`start_event_pipeline` writes `QueueIdle` to the rig-log only via a 500ms
`tokio::time::timeout` drain check *after* each strand event. The original
code had a single drain check that ran after the first event in the loop:

```
loop {
    event = debounce_rx.recv().await;   // blocking — waits forever
    process(event);
    match timeout(500ms, debounce_rx.recv()) {
        Ok(Some(next)) => { process(next); /* loops back to top */ },
        Ok(None) => break,
        Err(_) => write QueueIdle,
    }
}
```

When the drain check found `next` (event #2), it processed it and the
`loop { }` fell through back to the **blocking** `recv()` at the top.
No drain check ran after event #2. If no more events arrived, the loop
blocked forever and `QueueIdle` was never written.

This means:
- Burst of 1 event: `QueueIdle` written ✓
- Burst of 2 events: `QueueIdle` **never** written ✗ (blocks on top recv)
- Burst of 3+ events: `QueueIdle` **never** written ✗ (same)

In production, a strand file change fires events for multiple knots
(e.g. `arch-planner` + `coding-planner` both watch `project/plans/`),
so bursts of 2+ are the norm — `QueueIdle` was never written.

#### Fix

Replace the single `if` drain check with `is_burst_active` flag that
controls whether the next `recv` is blocking or timed:

```
loop {
    next = if is_burst_active {
        timeout(500ms, recv())  // drain check
    } else {
        recv()                  // blocking — wait for first event
    };

    match next {
        Some(event) => {
            is_burst_active = true;  // next recv will use timeout
            process(event);
        }
        None => break,
    }
}
```

- First event: `is_burst_active` is `false` → blocking `recv()` fires
- After processing: `is_burst_active = true` → next recv uses timeout
  - If another event arrives within 500ms: process it, loop continues
    (next recv still uses timeout — keeps draining)
  - If 500ms passes: `QueueIdle` written, `is_burst_active = false`,
    `continue` → next recv blocks again

#### Debug logging (temporary)

`eprintln!` statements added at every decision point for live debugging.
Remove before merging or gate behind `RUST_LOG`/debug flag.

- [x] Add `is_burst_active` flag controlling recv mode (blocking vs. timed)
- [x] Drain check now runs after *every* event (not just the second)
- [x] Add `eprintln!` debug logging at each decision point
- [x] Compile and run tests
- [x] Live test verified: `QueueIdle` written after burst of 2 events

---

## Test Summary

| # | Test | Type | File | Phase |
|---|------|------|------|-------|
| 1 | `config_handler_knot_modified_new_knot_registers` | Unit | `src/application/usecases.rs` | 0 |
| 2 | `config_handler_knot_modified_not_found` (updated) | Unit | `src/application/usecases.rs` | 0 |
| 3 | `http_post_loom_verifies_knot_registered` | Integration | `tests/auto_discovery_and_knot_crud.rs` | 1 |
| 4 | `filesystem_loom_creation_race_recovery` | Integration | `tests/auto_discovery_and_knot_crud.rs` | 2 |
| 5 | `knot_status_during_processing_does_not_hang` | Integration | `tests/pipeline.rs` | 3 |
| 6 | `single_event_queue_idle_written` | Integration | `tests/rig_log.rs` | 4 |
| 7 | `burst_events_single_queue_idle` | Integration | `tests/rig_log.rs` | 4 |
| 8 | `timeout_writes_rig_log_entry` | Integration | `tests/rig_log.rs` | 4 |

---

## Risks

- Phase 1 changes the error contract of `handle_knot_modified`. The existing test `config_handler_knot_modified_not_found` asserts an error is returned — this will need updating. The risk is low because the error was only useful for debugging; the recovery path is strictly better.
- Phase 4 may not reproduce the hang (it was observed once during live testing). If it doesn't reproduce, we still add `spawn_blocking` as a safety net since it costs virtually nothing and prevents any future blocking.
