# Plan: Fix KnotModified race and GET knot-status hang

**Status:** Draft
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

- [ ] Modify `handle_knot_modified` to treat missing knot as new registration: append knot to loom, log `KnotRegistered`, start watcher, emit warning
- [ ] Add unit test `config_handler_knot_modified_new_knot_registers`
- [ ] Add unit test `config_handler_knot_modified_warns_on_recovery`
- [ ] Update existing test `config_handler_knot_modified_not_found` to expect recovery
- [ ] Compile and run tests

### Phase 1: HTTP `POST /looms` verifies auto-discovery (integration test)

**File:** `tests/auto_discovery_and_knot_crud.rs`

Add an integration test that:
1. Starts server with empty rig
2. Calls `POST /looms` to create a loom with 1 knot
3. Verifies the loom is registered with the correct knot count (not 0)
4. Verifies the `.loom-log` contains `KnotRegistered` for the knot

This proves the HTTP creation path is resilient to the notify race — either the HTTP handler registers directly, or auto-discovery pre-registers and the HTTP handler is idempotent.

- [ ] Add integration test `http_post_loom_verifies_knot_registered` in `tests/auto_discovery_and_knot_crud.rs`
- [ ] Test verifies loom registered with correct knot count after `POST /looms`
- [ ] Test verifies `.loom-log` contains `KnotRegistered`
- [ ] Compile and run tests

### Phase 2: Filesystem loom creation race (integration test)

**File:** `tests/auto_discovery_and_knot_crud.rs`

Add an integration test that reproduces the race:
1. Start server with empty rig
2. Create a `*-loom/` directory
3. **Immediately** write the `.md` file (minimising time between dir creation and file write)
4. Poll `GET /looms/{id}` until the loom has the expected knot count

This proves the `KnotModified` recovery path works end-to-end. If the test consistently passes, the fix is verified.

- [ ] Add integration test `filesystem_loom_creation_race_recovery` in `tests/auto_discovery_and_knot_crud.rs`
- [ ] Test creates loom directory then immediately writes `.md` file
- [ ] Test polls `GET /looms/{id}` until expected knot count
- [ ] Compile and run tests

### Phase 3: Prove and fix knot-status read blocking

**File:** `tests/pipeline.rs` (or new `tests/knot_status_concurrent.rs`)

Add an integration test:
1. Start server with a loom + knot using a **slow mock agent** (e.g., `sleep 10` before writing output)
2. Create a strand to start processing
3. While the agent is mid-processing (agent is running, loom-log has `KnotProcessing` but not `KnotCompleted`), concurrently send 10 `GET /looms/{id}/knots/{name}` requests
4. Assert all requests complete within a reasonable timeout (e.g., 5 seconds)

If any request hangs/times out, the issue is confirmed. The fix is to wrap `read_all()` in `tokio::task::spawn_blocking` inside the handler.

- [ ] Add integration test `knot_status_during_processing_does_not_hang`
- [ ] Test uses slow mock agent, sends 10 concurrent knot-status requests
- [ ] Test asserts all requests complete within 5 seconds
- [ ] Apply `spawn_blocking` fix in `src/adapters/inbound/loom.rs` (as safety net even if not reproducible)
- [ ] Compile and run tests

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

## Test Summary

| # | Test | Type | File | Phase |
|---|------|------|------|-------|
| 1 | `config_handler_knot_modified_new_knot_registers` | Unit | `src/application/usecases.rs` | 0 |
| 2 | `config_handler_knot_modified_not_found` (updated) | Unit | `src/application/usecases.rs` | 0 |
| 3 | `http_post_loom_verifies_knot_registered` | Integration | `tests/auto_discovery_and_knot_crud.rs` | 1 |
| 4 | `filesystem_loom_creation_race_recovery` | Integration | `tests/auto_discovery_and_knot_crud.rs` | 2 |
| 5 | `knot_status_during_processing_does_not_hang` | Integration | `tests/pipeline.rs` or new file | 3 |

---

## Risks

- Phase 1 changes the error contract of `handle_knot_modified`. The existing test `config_handler_knot_modified_not_found` asserts an error is returned — this will need updating. The risk is low because the error was only useful for debugging; the recovery path is strictly better.
- Phase 4 may not reproduce the hang (it was observed once during live testing). If it doesn't reproduce, we still add `spawn_blocking` as a safety net since it costs virtually nothing and prevents any future blocking.
