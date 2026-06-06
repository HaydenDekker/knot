# ADR-001: Integration Test Server Pattern

**Date**: 2026-06-07
**Status**: Accepted

## Context

Knot's integration tests need to verify HTTP endpoints and the full event pipeline (file watch → debounce → agent → tie-off). This requires spinning up the server in-process during tests.

The original `ServerHandle` in `tests/helpers.rs` spawned the server in a `std::thread` with its own `tokio::runtime`, ran `rt.block_on(start_server_with_shutdown(...))`, then called `thread.join()` to wait for clean shutdown. This pattern causes the test runner to hang when the server thread doesn't exit.

### Root Cause: Runtime Drop Behaviour, Not `block_on`

The initial diagnosis was that `rt.block_on(...)` waited for all background tasks to complete. This is incorrect. `rt.block_on(...)` returns the exact millisecond the single future passed directly into it completes — which is `axum::serve().with_graceful_shutdown()`. When the shutdown signal fires, axum stops accepting connections, drains active requests, and the `block_on` future returns.

**The real hang occurs after `block_on` returns**, when the `rt` variable goes out of scope and its `Drop` implementation runs. By default, `tokio::runtime::Runtime`'s destructor **blocks the current thread indefinitely** until all spawned background tasks have been driven to completion or terminated. Because Knot's pipeline tasks (debounce engine, `ProcessStrand`, `ConfigEventHandler`) can sit in a `recv().await` loop or a debounce timeout, they may never finish — so the destructor blocks forever, the `std::thread` never exits, and `thread.join()` hangs.

```rust
std::thread::spawn(move || {
    let rt = tokio::runtime::Builder::new_multi_thread().build().unwrap();

    rt.block_on(async move {
        // Pipeline tasks spawned here (e.g. ProcessStrand) may never finish.

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await;
        // 1. Axum finishes. block_on returns.
    }); // 2. The block_on future scope closes.

    // 3. RIGHT HERE: `rt` goes out of scope and its Drop blocks
    //    forever waiting for pipeline tasks to complete.
});
```

This manifests as tests that pass locally but time out in CI, or the test runner hanging after test completion with "Server thread panicked" on forced teardown.

### Investigation

A standalone test suite (`tests/axum_server_test_integration.rs`) was created to isolate each pattern against axum's own testing example (`examples/testing/src/main.rs`). Six approaches were evaluated:

| Approach | Pattern | Hangs with blocking task? |
|---|---|---|
| 1 | `tokio::spawn`, no graceful shutdown | No — task is dropped at test end |
| 2 | `tokio::spawn` + shutdown signal + `time::timeout` | No — timeout bounds the wait |
| 3 | `std::thread` + `block_on` + oneshot + `thread.join()` | No (without extra tasks) |
| 3b | Same as 3 + `tokio::spawn(pending())` | **Yes** — reproduces Knot's problem |
| 3c | Same as 3b, but `drop(thread)` instead of join | No — thread is leaked |
| 3d | Same as 3b + `rt.shutdown_timeout(1s)` | No — abandons blocking tasks |
| 4 | `tower::Service::oneshot`, no server | N/A — no server lifecycle |

Approach 3b confirmed the hang at the `rt` destructor (not inside `block_on`). Approach 3d proved that `rt.shutdown_timeout()` fixes the hang by abandoning blocking tasks after a grace period.

## Decision

Use `tokio::test` + `tokio::spawn` without graceful shutdown for integration test server lifecycle. This is axum's recommended pattern from their own testing example.

### Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                   tokio runtime                      │
│                   (test harness)                     │
│                                                      │
│  ┌──────────────────────────────────────────────┐   │
│  │  axum::serve(listener, app)                  │   │
│  │  spawned via tokio::spawn, no graceful       │   │
│  │  shutdown — task drops when test ends        │   │
│  └──────────────────────────────────────────────┘   │
│                                                      │
│  ┌──────────────────────────────────────────────┐   │
│  │  Pipeline tasks (debounce, process, config)  │   │
│  │  spawned by start_event_pipeline /            │   │
│  │  start_config_pipeline — same runtime        │   │
│  └──────────────────────────────────────────────┘   │
│                                                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐          │
│  │ HTTP     │  │ Poll     │  │ Assert   │          │
│  │ client   │→│ port     │→│ tie-off  │          │
│  └──────────┘  └──────────┘  └──────────┘          │
└─────────────────────────────────────────────────────┘
```

The server, pipeline tasks, and test assertions all run on the same `tokio` runtime. When the test function returns, all tasks are dropped and the runtime cleans up.

### Implications for Design

- **No `std::thread` in test helpers** — `ServerHandle`, `spawn_server()`, and their shutdown semantics are removed from `tests/helpers.rs`.
- **No graceful shutdown in tests** — `start_server_with_shutdown` and `ShutdownSignal` remain for production use. Tests use the simpler `start_server` path or bind + spawn directly.
- **One runtime, not two** — the test runtime handles all async work. No `rt.block_on(...)` wrapping inside test threads.
- **Pipeline tasks must be cancellable** — they run on the same runtime as the test, so they are dropped when the test function returns. This means `while let Some(event) = rx.recv().await` loops terminate naturally when the channel sender is dropped, which happens when the `AppContext` is dropped at test end.
- **Port allocation** — use `TcpListener::bind("127.0.0.1:0")` to get a random available port, then read `listener.local_addr()` for the actual port. Avoids port conflicts between concurrent tests.

### Testing Strategy

Integration tests follow this pattern:

```rust
#[tokio::test]
async fn my_integration_test() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = tmp.path().join("rig");
    fs::create_dir(&rig).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let host_port = format!("127.0.0.1:{}", addr.port());

    let config = AppConfig {
        base_dir: rig.clone(),
        bind_addr: addr,
        ..AppConfig::default_config()
    };

    // Spawn and forget — no graceful shutdown.
    tokio::spawn(async move {
        let _ = knot::start_server(config).await;
    });

    // Wait for server, make assertions, test exits, server drops.
    wait_for_port(&host_port, 5000).await.unwrap();
    // ... HTTP assertions ...
}
```

The `wait_for_port` and HTTP helpers are async, running on the same runtime as the server.

### Dependencies

No new dependencies. Uses existing `tokio`, `axum`, and `tempfile` from dev-dependencies.

## Consequences

### Positive

- **Tests never hang on shutdown** — tasks are dropped by the runtime when the test function returns; no `thread.join()` waiting for a potentially blocked thread.
- **Faster test teardown** — no graceful shutdown drain phase in tests; the runtime simply drops all tasks.
- **Matches axum's own pattern** — follows the official `examples/testing/src/main.rs` guidance directly.
- **Simpler test helpers** — removes `ServerHandle`, `spawn_server()`, `send_shutdown()`, and the `ShutdownSignal::Channel` path from test code.
- **One runtime** — no thread boundary between test and server; debuggers and log output stay in one context.

### Negative

- **No clean shutdown in tests** — server tasks are forcibly dropped. This is acceptable for tests but means test shutdown behaviour doesn't exercise the production graceful shutdown path. The production path remains covered by manual testing and by the `start_server_with_shutdown` function's type-level contract.
- **All tests must be `#[tokio::test]`** — integration tests can no longer use sync `#[test]` with `std::thread` helpers. This is a constraint but not a limitation, since the server itself is async.
- **Port conflicts require discipline** — each test must bind its own listener on port 0 and pass the resolved address. Shared ports will cause race conditions between concurrent tests.
- **`helpers.rs` restructuring needed** — the existing sync-based helpers (`ServerHandle`, `spawn_server`) and the `ShutdownSignal::Channel` variant are no longer used by tests and should be removed.

### Trade-offs Considered

| Alternative | Rejected Because |
|---|---|
| `std::thread` + `block_on` + `thread.join()` (current) | Hangs at `rt` destructor when any background task blocks — unreliable in CI, hard to debug |
| `tokio::spawn` + shutdown signal + `time::timeout` | Adds complexity (timeout tuning, signal wiring) for no benefit over simple spawn. Timeout is a band-aid, not a fix. |
| `std::thread` + `block_on` + `drop(thread)` (no join) | Works but leaks threads on every test — resource exhaustion risk, unclear semantics |
| `std::thread` + `block_on` + `rt.shutdown_timeout()` (approach 3d) | Fixes the hang, but still requires a separate thread and runtime. More complexity than approach 1 with no benefit — the two-runtime model adds overhead and a thread boundary for no reason |
| `tower::Service::oneshot` (no server) | Excellent for handler/unit tests, but cannot test the full pipeline (file watch, debounce, agent CLI, tie-off) which requires a real server |
| Separate process per test (spawn `knot` binary) | Slow startup, no shared state, cannot inspect internals, overkill for in-process testing |

## Test Implementations (Reference)

The test code from `tests/axum_server_test_integration.rs` is preserved here as evidence of each approach's behaviour. Only Approaches 1 and 3d are retained in the test suite.

### Approach 1 — `tokio::spawn`, no graceful shutdown (chosen)

```rust
#[tokio::test]
async fn spawn_no_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn and forget — no graceful shutdown.
    tokio::spawn(async move {
        axum::serve(listener, simple_app()).await.unwrap();
    });

    let host_port = format!("127.0.0.1:{}", addr.port());
    wait_for_port_async(&host_port, 5000).await.unwrap();
    let response = http_get_async(&host_port, "/health").await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    // No explicit shutdown — task is dropped when test ends.
}
```

### Approach 2 — `tokio::spawn` + shutdown signal + `time::timeout`

```rust
#[tokio::test]
async fn spawn_shutdown_with_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, simple_app())
            .with_graceful_shutdown(async move { let _ = shutdown_rx.await; })
            .await
            .unwrap();
    });

    let host_port = format!("127.0.0.1:{}", addr.port());
    wait_for_port_async(&host_port, 5000).await.unwrap();
    let response = http_get_async(&host_port, "/health").await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));

    let _ = shutdown_tx.send(());
    let result = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(result.is_ok());
}
```

### Approach 3 — `std::thread` + `block_on` + oneshot + `thread.join()` (Knot's original)

```rust
fn spawn_server_in_thread(
    port: u16,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("create runtime");
        let addr: std::net::SocketAddr =
            format!("127.0.0.1:{port}").parse().unwrap();
        let _ = rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            axum::serve(listener, simple_app())
                .with_graceful_shutdown(async move { let _ = shutdown_rx.await; })
                .await
                .unwrap();
        });
        // rt drops here — no background tasks, so this is fine.
    })
}

#[test]
fn thread_runtime_oneshot_join() {
    let port = 35600;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = spawn_server_in_thread(port, shutdown_rx);
    wait_for_port(&format!("127.0.0.1:{port}"), 10000).unwrap();
    let response = http_get(&format!("127.0.0.1:{port}"), "/health").unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    let _ = shutdown_tx.send(());
    // thread.join() blocks until rt block_on returns, then rt drops.
    // Passes here (no extra tasks), but hangs with Knot's pipeline tasks.
}
```

### Approach 3b — Same as 3 + blocking background task (**demonstrates the hang**)

```rust
fn spawn_server_with_blocking_task(
    port: u16,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let addr: std::net::SocketAddr =
            format!("127.0.0.1:{port}").parse().unwrap();
        let _ = rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            // Simulates Knot's pipeline task that never drains:
            let _blocking_handle = tokio::spawn(async {
                futures_util::future::pending::<()>().await;
            });
            axum::serve(listener, simple_app())
                .with_graceful_shutdown(async move { let _ = shutdown_rx.await; })
                .await
                .unwrap();
        });
        // block_on returns here (axum is done).
        // rt drops here — blocks forever waiting for pending() task.
    })
}

#[test]
#[ignore = "hangs — reproduces Knot's problem"]
fn thread_with_blocking_task() {
    let port = 35601;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = spawn_server_with_blocking_task(port, shutdown_rx);
    // ... wait, assert ...
    let _ = shutdown_tx.send(());
    // thread.join() hangs: rt's Drop blocks forever on the pending() task.
}
```

### Approach 3c — Same as 3b, `drop(thread)` instead of join

```rust
#[test]
fn thread_with_timeout_shutdown() {
    let thread = spawn_server_with_blocking_task(port, shutdown_rx);
    // ... assertions ...
    let _ = shutdown_tx.send(());
    drop(thread); // Don't join — thread is leaked on exit.
}
```

### Approach 3d — Same as 3b + `rt.shutdown_timeout()` (**fixes the hang**)

```rust
fn spawn_server_with_shutdown_timeout(
    port: u16,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("create runtime");

        let addr: std::net::SocketAddr =
            format!("127.0.0.1:{port}").parse().unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            let _blocking_handle = tokio::spawn(async {
                futures_util::future::pending::<()>().await;
            });
            axum::serve(listener, simple_app())
                .with_graceful_shutdown(async move { let _ = shutdown_rx.await; })
                .await
                .unwrap();
        });

        // Explicitly shut down the runtime with a 1-second grace period
        // instead of letting the default Drop block forever.
        rt.shutdown_timeout(Duration::from_secs(1));
    })
}

#[test]
fn server_thread_shutdown_timeout() {
    let port = 35610;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = spawn_server_with_shutdown_timeout(port, shutdown_rx);
    // ... wait, assert ...
    let _ = shutdown_tx.send(());
    // thread.join() returns: rt.shutdown_timeout abandoned the blocking task.
}
```

### Approach 4 — `tower::Service::oneshot`, no server at all

```rust
#[tokio::test]
async fn tower_oneshot_no_server() {
    use axum::body::Body;
    use http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let app = simple_app();
    let response = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap();
    assert_eq!(&body.to_bytes()[..], b"ok");
}
```

## References

- axum `examples/testing/src/main.rs` — official testing example showing `tokio::spawn(axum::serve(listener, app()).await)`
- axum `axum/src/serve/mod.rs` — `with_graceful_shutdown` implementation and drain semantics
- tokio `tokio/src/runtime/runtime.rs` — `Runtime` `Drop` implementation (blocks until all tasks complete) and `shutdown_timeout` (abandons tasks after grace period)
- `tests/helpers.rs` — current `ServerHandle` and `spawn_server()` implementation (to be replaced)
- `src/lib.rs` — `start_server_with_shutdown`, `ShutdownSignal`, pipeline startup
- `tests/axum_server_test_integration.rs` — living test reference (approach 1 + approach 3d)
