# Plan: Migrate Integration Tests to tokio::spawn Server Pattern

## Related PRD

None — this is a refactor, not a feature.

## Problem

Integration tests spawn the Knot server in a `std::thread` with its own `tokio::runtime`, then call `rt.block_on(start_server_with_shutdown(...))`. When `rt` is dropped after `block_on` returns, its `Drop` implementation blocks indefinitely waiting for all spawned background tasks to complete. Pipeline tasks (debounce, ProcessStrand) sit in `recv().await` loops that never terminate — `thread.join()` hangs forever.

[ADR-001](../adrs/adr-001-integration-test-server-pattern.md) documents the investigation and decision: use `tokio::spawn` on the test harness runtime with no graceful shutdown.

## Target

All 12 test files that currently use `#[test]` + `spawn_server(config)` + `shutdown.stop()` migrate to `#[tokio::test]` + `tokio::spawn(knot::start_server(config))`. The server task is dropped when the test function returns — no explicit shutdown call. `helpers.rs` provides async HTTP helpers and a `spawn_server` that returns `tokio::task::JoinHandle<()>`. `ServerHandle` and the old `spawn_server` are removed.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test File | Tests | Current Pattern |
|-----------|-------|-----------------|
| `helpers.rs` | 0 (infra) | `ServerHandle`, sync `spawn_server`, sync HTTP helpers |
| `server_startup_smoke.rs` | 3 | `#[test]`, `spawn_server`, `.stop()` |
| `discovery.rs` | 4 | `#[test]`, `spawn_server`, `.stop()` |
| `rig_lifecycle.rs` | 5 | `#[test]`, `spawn_server`, `.stop()` |
| `loom_crud.rs` | 2 | `#[test]`, `spawn_server`, `.stop()` |
| `multi_loom.rs` | 2 | `#[test]`, `spawn_server`, `.stop()` |
| `demo.rs` | 2 | `#[test]`, `spawn_server`, `.stop()` |
| `agent_integration.rs` | 3 | `#[test]`, `spawn_server`, `.stop()` |
| `pipeline.rs` | 5 | `#[test]`, `spawn_server`, `.stop()` |
| `auto_discovery_and_knot_crud.rs` | 8 | `#[test]`, `spawn_server`, `.stop()` |
| `tie_off.rs` | 2 | `#[test]`, `spawn_server`, `.stop()` |
| `shutdown.rs` | 2 | `#[test]`, `spawn_server`, `.stop()` |

**Total: 38 tests across 12 files + 1 helper rewrite.**

Already migrated (no change): `axum_server_test_integration.rs`, `composition.rs`, `filesystem_interface.rs`, `http_interface.rs`, `skill_integration.rs`, `swagger_ui.rs`

## Test Gaps

None — this migration preserves existing test coverage. Only the server lifecycle mechanism changes.

## Phases

### Phase 0: helpers.rs — Async helpers and new spawn_server

- [ ] Add `async fn wait_for_port(host_port: &str, timeout_ms: u64) -> Result<(), String>` using `tokio::net::TcpStream::connect`
- [ ] Add `async fn http_get(host_port: &str, path: &str) -> Result<(String, String), String>` using `tokio::io::{AsyncReadExt, AsyncWriteExt}`
- [ ] Add `async fn http_get_retry(host_port: &str, path: &str, retries: usize, delay_ms: u64) -> Result<(String, String), String>`
- [ ] Add `async fn http_post_json(host_port: &str, path: &str, body: &serde_json::Value) -> Result<(String, String), String>`
- [ ] Add `async fn http_post_json_retry(host_port: &str, path: &str, body: &serde_json::Value, retries: u32, delay_ms: u64) -> Result<(String, String), String>`
- [ ] Add `async fn http_delete(host_port: &str, path: &str) -> Result<(String, String), String>`
- [ ] Add `fn spawn_server(config: AppConfig) -> tokio::task::JoinHandle<()>`:
  ```rust
  pub fn spawn_server(config: AppConfig) -> tokio::task::JoinHandle<()> {
      tokio::spawn(async move {
          let _ = knot::start_server(config).await;
      })
  }
  ```
- [ ] Remove `ServerHandle` struct and its `impl` block
- [ ] Remove old `fn spawn_server(config) -> ServerHandle`
- [ ] Remove `use knot::ShutdownSignal`
- [ ] Remove sync helpers: `http_get`, `http_get_retry`, `http_post_json`, `http_patch_json`, `http_post_json_retry`, `http_delete_retry`, `http_delete`, sync `wait_for_port` — verify no remaining callers first
- [ ] Retain: `make_knot_content_with_dirs`, `make_knot_content`, `create_mock_agent`, `create_stub_pi_agent`, `wait_for_file`, `poll_knot_status`
- [ ] Run `cargo test --test axum_server_test_integration` — both tests must pass

**Stop condition:** If `axum_server_test_integration` fails, the async helpers or new `spawn_server` are broken — do not proceed.

### Phase 1: server_startup_smoke.rs

**Primary regression target** — this is the test file that originally demonstrated the hang.

- [ ] Convert all 3 tests: `#[test]` → `#[tokio::test] async fn`
- [ ] Replace `let port = NNNNN` → `TcpListener::bind("127.0.0.1:0").await.unwrap()`, `let addr = listener.local_addr().unwrap()`
- [ ] Replace `bind_addr: format!("127.0.0.1:{port}").parse().unwrap()` → `bind_addr: addr`
- [ ] Replace `spawn_server(config)` → `spawn_server(config)` (new signature, returns `JoinHandle`)
- [ ] Replace `wait_for_port(&host_port, N, N)` → `wait_for_port(&host_port, 5000).await`
- [ ] Replace `http_get(...)` → `http_get(...).await`
- [ ] Replace `format!("127.0.0.1:{port}")` → `format!("127.0.0.1:{}", addr.port())`
- [ ] Remove all `.stop()` calls
- [ ] Run `cargo test --test server_startup_smoke` — all 3 must pass

**Stop condition:** If any failure relates to axum server startup or connection, stop and debug. Other failures noted but continue.

### Phase 2: discovery.rs

- [ ] Convert 4 tests (same mechanical changes as Phase 1)
- [ ] Run `cargo test --test discovery` — all 4 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 3: rig_lifecycle.rs

- [ ] Convert 5 tests
- [ ] Run `cargo test --test rig_lifecycle` — all 5 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 4: loom_crud.rs

- [ ] Convert 2 tests
- [ ] Run `cargo test --test loom_crud` — all 2 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 5: multi_loom.rs

- [ ] Convert 2 tests
- [ ] Run `cargo test --test multi_loom` — all 2 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 6: demo.rs

- [ ] Convert 2 tests
- [ ] Run `cargo test --test demo` — all 2 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 7: agent_integration.rs

- [ ] Convert 3 tests
- [ ] Run `cargo test --test agent_integration` — all 3 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 8: pipeline.rs

- [ ] Convert 5 tests
- [ ] Run `cargo test --test pipeline` — all 5 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 9: auto_discovery_and_knot_crud.rs

**Largest test file** (8 tests, 804 lines) — placed late so the migration pattern is well-validated.

- [ ] Convert 8 tests
- [ ] Run `cargo test --test auto_discovery_and_knot_crud` — all 8 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 10: tie_off.rs

- [ ] Convert 2 tests
- [ ] Run `cargo test --test tie_off` — all 2 must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 11: shutdown.rs

**Semantic change:** This file tests shutdown behaviour. Under the new pattern, the server task is dropped when the test function returns — no explicit `.stop()` call. Tests that verify post-shutdown state (e.g., tie-off files are not written after shutdown) must restructure: verify state before the test function returns, or accept that graceful shutdown is a production-only path.

- [ ] Convert 2 tests
- [ ] Review assertions that depend on `.stop()` semantics — adjust or document limitations
- [ ] Run `cargo test --test shutdown` — tests must pass
- [ ] Regression: `cargo test --test server_startup_smoke`

### Phase 12: Full suite verification and cleanup

- [ ] Run `cargo test` — full suite, all tests pass
- [ ] Run `cargo clippy --tests` — no new warnings
- [ ] Verify no dead code in helpers.rs
- [ ] Check `src/lib.rs`: if `ShutdownSignal::Channel` is only used by tests, remove it and collapse `ShutdownSignal` to a unit struct
- [ ] Final `cargo test` — all tests pass, 0 failures, 0 warnings

## Per-Phase Migration Template

Each test file phase (except Phase 0, 11, 12) follows this mechanical transformation:

```diff
-#[test]
+#[tokio::test]
+async
fn test_name() {
     let tmp = tempfile::tempdir().unwrap();
     // ... rig/loom/strand setup ...

-    let port = NNNNN;
+    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
+    let addr = listener.local_addr().unwrap();
     let config = AppConfig {
         base_dir: rig,
-        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
+        bind_addr: addr,
         ..AppConfig::default_config()
     };

-    let shutdown = spawn_server(config);
-    let host_port = format!("127.0.0.1:{port}");
+    let _handle = spawn_server(config);
+    let host_port = format!("127.0.0.1:{}", addr.port());

-    wait_for_port(&host_port, 100, 50).expect("server should start");
+    wait_for_port(&host_port, 5000).await.expect("server should start");

     // ... assertions ...
-    // sync helpers → async helpers + .await:
-    http_get_retry(&host_port, "/path", 30, 100)
+    http_get_retry(&host_port, "/path", 30, 100).await

-    shutdown.stop();
+    // No explicit shutdown — task is dropped when test ends.
 }
```
