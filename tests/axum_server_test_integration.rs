//! Axum server test pattern reference — `tokio::spawn`, no graceful shutdown.
//!
//! This is the accepted pattern per [ADR-001](../project/adrs/adr-001-integration-test-server-pattern.md).
//!
//! Server is spawned and forgotten; the test runtime cleans up tasks when
//! the test function returns. No graceful shutdown, no `std::thread`, no
//! `rt.block_on` — just `tokio::spawn` on the shared test runtime.
//!
//! An additional test (approach 3d) proves that `rt.shutdown_timeout()` fixes
//! the hang from the old `std::thread` + `block_on` pattern. See the ADR for
//! the full technical explanation.

use std::net::TcpStream;
use std::time::Duration;

use axum::{routing::get, Router};
use tokio::net::TcpListener;

// ── Shared helpers ──────────────────────────────────────────────────────

fn simple_app() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

/// Poll a port until it accepts a TCP connection or timeout (async).
async fn wait_for_port_async(host_port: &str, timeout_ms: u64) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if tokio::net::TcpStream::connect(host_port).await.is_ok() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("port {host_port} did not open in {timeout_ms}ms"));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Async HTTP GET via raw TCP.
async fn http_get_async(host_port: &str, path: &str) -> Result<String, String> {
    let mut stream = tokio::net::TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n"
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(request.as_bytes()).await.map_err(|e| format!("write failed: {e}"))?;
    stream.flush().await.map_err(|e| format!("flush failed: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("read failed: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Poll a port until it accepts a TCP connection or timeout (sync).
fn wait_for_port_sync(host_port: &str, timeout_ms: u64) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if TcpStream::connect(host_port).is_ok() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("port {host_port} did not open in {timeout_ms}ms"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Simple HTTP GET via raw TCP (sync).
fn http_get_sync(host_port: &str, path: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(host_port)
        .map_err(|e| format!("connect failed: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n"
    );
    use std::io::{Read, Write};
    stream.write_all(request.as_bytes()).map_err(|e| format!("write failed: {e}"))?;
    stream.flush().map_err(|e| format!("flush failed: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| format!("read failed: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

// ── Approach 1 — tokio::spawn, no graceful shutdown (chosen) ────────────

/// Reference implementation of the accepted server test pattern.
///
/// Pattern:
/// 1. Bind `TcpListener` on port 0 (random available port).
/// 2. `tokio::spawn(axum::serve(listener, app).await)` — no graceful shutdown.
/// 3. Wait for port to open, make HTTP assertions.
/// 4. Test function returns → tasks are dropped → runtime cleans up.
///
/// See [ADR-001](../project/adrs/adr-001-integration-test-server-pattern.md)
/// for the full rationale and rejected alternatives.
#[tokio::test]
async fn server_spawn_no_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn and forget — no graceful shutdown.
    tokio::spawn(async move {
        axum::serve(listener, simple_app()).await.unwrap();
    });

    let host_port = format!("127.0.0.1:{}", addr.port());
    wait_for_port_async(&host_port, 5000).await.expect("server should start");

    let response = http_get_async(&host_port, "/health")
        .await
        .expect("health should respond");
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "expected 200, got: {response}"
    );

    // No explicit shutdown needed — task is dropped when the test ends.
}

// ── Approach 3d — std::thread + block_on + shutdown_timeout ─────────────
//
// Proves that `rt.shutdown_timeout()` fixes the hang from Knot's original
// pattern. The real culprit is not `rt.block_on` (which returns as soon as
// the serve future completes) — it is the `rt` destructor, which by default
// blocks indefinitely waiting for all spawned background tasks to finish.
//
// `rt.shutdown_timeout(Duration::from_secs(1))` abandons any tasks still
// running after the grace period, allowing the thread to exit cleanly.

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

        let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

            // Simulate a pipeline task that never drains.
            let _blocking_handle = tokio::spawn(async {
                futures_util::future::pending::<()>().await;
            });

            axum::serve(listener, simple_app())
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        // Explicitly shut down the runtime with a short grace period
        // instead of letting the default `Drop` block forever.
        rt.shutdown_timeout(Duration::from_secs(1));
    })
}

/// Demonstrates that `rt.shutdown_timeout()` prevents the hang that
/// Knot's original pattern produces. This test passes where 3b (without
/// `shutdown_timeout`) hangs indefinitely.
///
/// The hang in the original pattern occurs at `rt`'s destructor — not inside
/// `rt.block_on`. When `block_on` returns (axum has finished), `rt` goes out
/// of scope and its `Drop` implementation blocks until all spawned tasks
/// complete. `shutdown_timeout` replaces the implicit drop with explicit
//  control, abandoning hanging tasks after the grace period.
#[test]
fn server_thread_shutdown_timeout() {
    let port = 35610;
    let host_port = format!("127.0.0.1:{port}");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = spawn_server_with_shutdown_timeout(port, shutdown_rx);

    wait_for_port_sync(&host_port, 10000).expect("server should start");

    let response = http_get_sync(&host_port, "/health").expect("health should respond");
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "expected 200, got: {response}"
    );

    // Signal shutdown — axum finishes, block_on returns.
    let _ = shutdown_tx.send(());

    // Thread exits cleanly because `rt.shutdown_timeout(1s)` abandons the
    // blocking task instead of waiting forever.
    let result = thread.join();
    assert!(
        result.is_ok(),
        "thread should exit cleanly with shutdown_timeout"
    );
}
