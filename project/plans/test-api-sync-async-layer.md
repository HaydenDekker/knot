# Plan: Sync Integration Tests to Async Layer (ADR-002/003)

## Problem

The async layer was restructured per [ADR-002](../adrs/adr-002-server-child-tasks.md) and [ADR-003](../adrs/adr-003-channel-cascade-shutdown.md): tasks are now spawned into a `JoinSet`, shutdown is cooperative via channel-cascade drain, and `spawn_server_with_shutdown()` returns a `(JoinHandle, oneshot::Sender)` tuple.

**8 test files still reference the old API** and fail to compile. The old pattern:

```rust
let shutdown = spawn_server(config);       // returned JoinHandle (no shutdown channel)
wait_for_port(&host_port, 100, 50);        // 3 args (old signature)
let _ = shutdown.send(());                 // compile error — JoinHandle has no .send()
http_get(&host_port, "/x").expect(...);     // missing .await on async fn
```

The new API in `helpers.rs`:

```rust
let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
wait_for_port(&host_port, 5000).await;       // 2 args, async
let _ = shutdown_tx.send(());                // oneshot::Sender
http_get(&host_port, "/x").await.expect(...); // .await required
```

All 8 failing files share the same 4 compile errors (see "Error Patterns" below). No logic changes are needed — only API signature updates.

## Target

All 8 test files compile and pass. The full integration test suite produces:

- `generic_task_management.rs` — 10 passed (already green, unchanged)
- `task_management.rs` — 4 passed, 1 ignored (already green, unchanged)
- 8 previously-failing files — all compile and pass

The helper signatures in `helpers.rs` are **not** changed. The failing tests are updated to match the helpers that already exist.

## Implementation Status: ✅ Complete (2026-06-08)

## ADR Constraints

This plan **must** adhere to [ADR-002](../adrs/adr-002-server-child-tasks.md) and [ADR-003](../adrs/adr-003-channel-cascade-shutdown.md). If implementation reveals a conflict (e.g. a test requires behaviour the ADRs explicitly reject like `tokio::spawn` without `JoinSet`), **stop and request an ADR change** before proceeding.

### What the ADRs require:

| ADR | Requirement | Test Impact |
|-----|-------------|-------------|
| ADR-002 | Tasks spawned into `JoinSet`, not `tokio::spawn` | `spawn_server_with_shutdown()` is the only path — its `JoinSet` owns pipeline tasks |
| ADR-002 | `while let Some = join_next()` drain loop, not single call | Tests must allow sufficient time for full drain (not just one task) |
| ADR-002 | No leaked channel senders | Tests must not hold extra `Sender` clones across shutdown |
| ADR-002 | Notify thread delay acknowledged | `in_flight_processing_completes_on_shutdown` remains `#[ignore]` |
| ADR-003 | Channel closure is shutdown signal | `shutdown_tx.send(())` is the correct trigger — oneshot → axum stops → AppContext drops → cascade |
| ADR-003 | Post-shutdown hooks after drain | Tests that check `LoomStopped` must wait for full server completion, not just signal send |

## Error Patterns (Root Cause)

All 8 failing files share these 4 errors:

| # | Error | Old Pattern | New Pattern |
|---|-------|-------------|-------------|
| 1 | `spawn_server` returns `JoinHandle` (no shutdown channel) | `let shutdown = spawn_server(config);` | `let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);` |
| 2 | `.send(())` on `JoinHandle` — no method exists | `let _ = shutdown.send(());` | `let _ = shutdown_tx.send(());` |
| 3 | `wait_for_port` takes 3 args but function now takes 2 | `wait_for_port(&host_port, 100, 50)` | `wait_for_port(&host_port, 5000).await` |
| 4 | Missing `.await` on async HTTP helpers | `http_get(&host_port, "/x").expect(...)` | `http_get(&host_port, "/x").await.expect(...)` |

## Occurrence Count Per File

| File | #1 spawn | #2 send | #3 port | #4 await | Tests Affected |
|------|----------|---------|---------|----------|----------------|
| `agent_integration.rs` | 3 | 3 | 3 | 4 | 3 tests |
| `discovery.rs` | 4 | 4 | 4 | 7 | 4 tests |
| `loom_crud.rs` | 3 | 3 | 3 | 6 | 3 tests |
| `multi_loom.rs` | 2 | 2 | 2 | 4 | 2 tests |
| `pipeline.rs` | 5 | 5 | 5 | 6 | 5 tests |
| `rig_lifecycle.rs` | 5 | 6* | 6 | 7 | 5 tests |
| `shutdown.rs` | 2 | 2 | 2 | 2 | 2 tests |
| `tie_off.rs` | 2 | 2 | 2 | 0 | 2 tests |

*\*rig_lifecycle.rs uses `shutdown1`/`shutdown2` variable names (two servers in one test)*

**Totals: 25 spawn fixes, 27 send fixes, 27 port fixes, 36 await fixes = ~115 line edits across 8 files.**

## Existing Tests

| Test File | What it covers | Status |
|-----------|---------------|--------|
| `tests/generic_task_management.rs` | Channel-cascade shutdown pattern (10 tests, tokio-only) | ✅ 10 passed |
| `tests/task_management.rs` | Full server cascade shutdown (5 tests, 1 ignored) | ✅ 4 passed, 1 ignored |
| `tests/helpers.rs` | Shared test infrastructure (spawn, HTTP, fixtures) | ✅ No test — module |
| `tests/agent_integration.rs` | Agent execution, error handling via subprocess | ❌ Compile error |
| `tests/discovery.rs` | Loom discovery, config loading | ❌ Compile error |
| `tests/loom_crud.rs` | Register/unregister looms via HTTP | ❌ Compile error |
| `tests/multi_loom.rs` | Multi-loom and multi-knot scenarios | ❌ Compile error |
| `tests/pipeline.rs` | Event pipeline end-to-end (create → debounce → process → tie-off) | ❌ Compile error |
| `tests/rig_lifecycle.rs` | Rig config, server restart, persistence | ❌ Compile error |
| `tests/shutdown.rs` | Basic shutdown behaviour (pre-cascade) | ❌ Compile error |
| `tests/tie_off.rs` | Tie-off creation and content verification | ❌ Compile error |

## Test Gaps

None — all behaviour is already tested. The failing tests define the correct expectations; they just use the wrong API signatures.

## Phases

### Phase 1: `shutdown.rs` — Baseline (2 tests)

Smallest file, most directly related to the async layer. Validates the pattern works before touching larger files.

- [x] Replace `spawn_server(config)` → `spawn_server_with_shutdown(config)` (2 occurrences)
- [x] Replace `shutdown.send(())` → `shutdown_tx.send(())` (2 occurrences)
- [x] Replace `wait_for_port(&host_port, 100, 50)` → `wait_for_port(&host_port, 5000).await` (2 occurrences)
- [x] Add `.await` to `http_get()` calls (2 occurrences)
- [x] Run `cargo test --test shutdown` — verify both tests pass
- [x] Confirm tests validate ADR-002 cascade (LoomStopped written after drain)

**Gate:** If `shutdown.rs` tests fail, the helpers or `lib.rs` may have a deeper issue. Stop and diagnose.

### Phase 2: `tie_off.rs` — Simple pipeline (2 tests)

No missing `.await` calls (simplest remaining file). Validates tie-off creation still works with the new spawn pattern.

- [x] Replace `spawn_server(config)` → `spawn_server_with_shutdown(config)` (2)
- [x] Replace `shutdown.send(())` → `shutdown_tx.send(())` (2)
- [x] Replace `wait_for_port(&host_port, 100, 50)` → `wait_for_port(&host_port, 5000).await` (2)
- [x] Run `cargo test --test tie_off` — verify both tests pass

### Phase 3: `multi_loom.rs` — Multi-loom scenarios (2 tests)

Validates multi-loom registration and multi-knot processing with the new pattern.

- [x] Apply all 4 fixes (2+2+2+4 = 10 line edits)
- [x] Run `cargo test --test multi_loom` — verify both tests pass

### Phase 4: `loom_crud.rs` — HTTP CRUD (3 tests)

Validates register/unregister/discover looms via HTTP with the new pattern.

- [x] Apply all 4 fixes (3+3+3+6 = 15 line edits)
- [x] Run `cargo test --test loom_crud` — verify all 3 tests pass

### Phase 5: `agent_integration.rs` — Agent execution (3 tests)

Validates subprocess agent invocation and error handling with the new pattern.

- [x] Apply all 4 fixes (3+3+3+4 = 13 line edits)
- [x] Run `cargo test --test agent_integration` — verify all 3 tests pass

### Phase 6: `discovery.rs` — Loom discovery (4 tests)

Validates rig scanning, config loading, and loom registration at startup.

- [x] Apply all 4 fixes (4+4+4+7 = 19 line edits)
- [x] Run `cargo test --test discovery` — verify all 4 tests pass

### Phase 7: `rig_lifecycle.rs` — Server lifecycle (5 tests)

Most complex — uses two servers in one test (`shutdown1`/`shutdown2`). Validates server restart and persistence.

- [x] Apply all 4 fixes (5+6+6+7 = 24 line edits)
- [x] Handle `shutdown1`/`shutdown2` variable naming carefully (two `spawn_server_with_shutdown` calls in one test)
- [x] Run `cargo test --test rig_lifecycle` — verify all 5 tests pass

### Phase 8: `pipeline.rs` — Full pipeline (5 tests)

Largest test file — event pipeline end-to-end. Validates debounce, processing, tie-off, and activity log.

- [x] Apply all 4 fixes (5+5+5+6 = 21 line edits)
- [x] Run `cargo test --test pipeline` — verify all 5 tests pass

### Phase 9: Full suite validation

- [x] Run full test suite: `cargo test --test '*'` (or `cargo test --tests`)
- [x] Verify expected results: all compile, expected pass/ignore count matches
- [x] Confirm no regressions in already-green tests (`generic_task_management`, `task_management`, `axum_server_test_integration`, `composition`, `demo`, `filesystem_interface`, `http_interface`, `server_startup_smoke`, `skill_integration`, `swagger_ui`)

## Notes

- All changes are **syntactic API updates only** — no test logic or assertions change
- Each phase validates independently, so failures are isolated to one file
- The `#[ignore]` test in `task_management.rs` (`in_flight_processing_completes_on_shutdown`) is intentionally kept as-is per ADR-002's documented notify thread limitation
- If any phase reveals a behavioural regression (test compiles but fails), it indicates the async layer implementation has diverged from the ADRs — stop and investigate before proceeding
