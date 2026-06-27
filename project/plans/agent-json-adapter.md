# Plan 46: JSON-based Agent Adapter

## Related PRD

This plan contributes to [Demand Control — Concurrency, Throughput and Service Tuning](../prds/prd-demand-control.md).

It provides the foundation for capturing session IDs and token usage from Pi invocations — a prerequisite for invocation performance visibility (Story 2), token usage tracking (Story 3), and session resume on failure (Plan 47, System Reliability Story 5a).

## Problem

Knot invokes Pi via `--print` mode, which outputs plain text to stdout. The only data captured is the agent's response string (`stdout`) and exit code. Session IDs, token usage, and invocation metadata are all lost.

Pi supports `--mode json` which outputs JSON-L (newline-delimited JSON) containing:
- Session ID in the first line (`{"type":"session","id":"..."}`)
- Token usage in `agent_end` event (`input`, `output`, `cacheRead`, `cacheWrite`, `totalTokens`)
- Response text in the final `message_end` content

Currently, Knot cannot access any of this data because it uses `--print` mode and treats stdout as an opaque string.

## Target

A new `PiJsonAgentRunner` adapter that invokes Pi with `--mode json`, parses the JSON-L stream, and extracts session ID + token usage metadata alongside the response text. The existing `SubprocessAgentRunner` (renamed to `PiStdioAgentRunner`) remains unchanged.

The rig config selects which adapter via `agent_adapter` — a simple enum with no invocation details:

```yaml
agent-adapter: pi-json    # or "pi-stdio" (default)
```

No `cli_path`, no `cli_args`. Each adapter hardcodes its own binary path and flags.

ADR-009 documents the decision to use agent-specific adapters rather than a generic CLI wrapper.

When `agent_adapter: pi-json` is configured:
- `AgentOutput` gains `metadata: Option<AgentInvocationMetadata>` containing `session_id` and `token_usage`
- `PortError` variants (`Timeout`, `AgentExecutionFailed`) carry an optional `session_id` for session resume
- The response text is extracted from the JSON-L `message_end` event

When `agent_adapter: pi-stdio` (default), behaviour is unchanged — `metadata` is `None`.

## Implementation Status: ✅ Complete

**Completed:** 2026-06-27
**Version:** 0.19.0 (MINOR — new feature, backwards compatible)

**Bugfix:** 2026-06-27 — `run_startup()` now auto-creates `.workspace-agent-config.yaml` on rig creation (default + named rigs) with `agent-adapter: pi-stdio` and a comment listing available adapters.

**Result:** 6 phases complete. `AgentInvocationMetadata` + `TokenUsage` structs in domain, `session_id` on `PortError::Timeout`/`AgentExecutionFailed`, `AgentAdapter` enum (`PiStdio`/`PiJson`) replacing `cli_path`/`cli_args` in `RigAgentConfig`. `PiJsonAgentRunner` parses JSON-L line-by-line, extracting session ID + token usage. `SubprocessAgentRunner` renamed to `PiStdioAgentRunner`. Adapter selection in composition root. 3 new integration tests + 12 new unit tests + 2 startup tests. Domain glossary updated. 612+ tests pass, clippy clean. ADR-009 documents the decision.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `src/adapters/subprocess.rs` | Subprocess spawn, stdout/stderr capture, timeout, non-zero exit, prompt passthrough, event metadata | ✅ Green — 14 tests |
| `tests/pipeline.rs` | Full event pipeline (file → debounce → process → tie-off) | ✅ Green — integration tests |
| `tests/git_versioning.rs` | Git commit after tie-off write | ✅ Green |
| `tests/profile_timeout.rs` | Timeout enforcement per agent profile | ✅ Green |
| `tests/auto_discovery_and_knot_crud.rs` | Dynamic loom/knot registration | ✅ Green |
| `src/application/ports.rs` | PortError variants, AgentRunner trait, MockAgentRunner | ✅ Green |
| `src/domain/value_objects.rs` | AgentConfig, RigAgentConfig construction and serialization | ✅ Green — 32 tests |

## Test Gaps

- No test for JSON-L parsing of Pi output
- No test for session ID extraction from Pi output
- No test for token usage extraction from Pi output
- No test for `agent_adapter` selection in rig config
- No integration test for `--mode json` end-to-end with real `pi` binary
- No regression test that stdio adapter still works when `agent_adapter: pi-stdio`
- No test for `PortError::is_resumable()` classification

## Phases

### Phase 0: Domain — New Types in Ports

Work in `src/application/ports.rs`.

**Changes:**

1. **Add `AgentInvocationMetadata` and `TokenUsage` structs:**
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   pub struct AgentInvocationMetadata {
       pub session_id: Option<String>,
       pub token_usage: Option<TokenUsage>,
   }

   #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
   pub struct TokenUsage {
       pub input: u64,
       pub output: u64,
       pub cache_read: u64,
       pub cache_write: u64,
       pub total: u64,
   }
   ```

2. **Add `metadata` field to `AgentOutput`:**
   ```rust
   pub struct AgentOutput {
       pub stdout: String,
       pub stderr: String,
       pub exit_code: i32,
       #[serde(default, skip_serializing_if = "Option::is_none")]
       pub metadata: Option<AgentInvocationMetadata>,
   }
   ```
   Default: `None`. Backwards compatible — existing callers that read `.stdout` continue to work.

3. **Add `session_id` to `PortError` variants (for Plan 47 session resume):**
   ```rust
   pub enum PortError {
       // ... existing variants ...
       AgentExecutionFailed { message: String, session_id: Option<String> },
       CommandNotFound(String),
       Timeout { message: String, session_id: Option<String> },
       // ...
   }
   ```
   This changes the shape of `Timeout` and `AgentExecutionFailed` from newtype to struct variant. Breaking for all callers — must update all construction sites.

4. **Add helper methods to `PortError`:**
   ```rust
   impl PortError {
       /// Extract session_id from errors that carry one.
       pub fn session_id(&self) -> Option<&String> {
           match self {
               PortError::Timeout { session_id, .. }
               | PortError::AgentExecutionFailed { session_id, .. }
                   => session_id.as_ref(),
               _ => None,
           }
       }

       /// Classify error as resumable (session can be retried) or fatal.
       pub fn is_resumable(&self) -> bool {
           matches!(
               self,
               PortError::Timeout { .. }
                   | PortError::AgentExecutionFailed { .. }
           )
       }
   }
   ```

5. **Update `PortError::Display` impl** for new struct variants.

**Tests (domain/application unit):**
- `test_agent_output_with_metadata()` — serialisation/deserialisation with metadata
- `test_agent_output_without_metadata()` — `metadata: None` round-trips correctly
- `test_token_usage_fields()` — TokenUsage fields are correct after deserialisation
- `test_port_error_session_id_timeout()` — `Timeout` variant returns session_id
- `test_port_error_session_id_execution_failed()` — `AgentExecutionFailed` returns session_id
- `test_port_error_session_id_command_not_found()` — `CommandNotFound` returns `None`
- `test_port_error_is_resumable_timeout()` — `Timeout` → `true`
- `test_port_error_is_resumable_execution_failed()` — `AgentExecutionFailed` → `true`
- `test_port_error_is_resumable_command_not_found()` — `CommandNotFound` → `false`
- `test_port_error_display_timeout_with_session()` — Display includes message, ignores session_id

**Existing tests to update (breaking change — PortError shape):**
- Every construction of `PortError::Timeout(msg)` → `PortError::Timeout { message: msg, session_id: None }`
- Every construction of `PortError::AgentExecutionFailed(msg)` → `PortError::AgentExecutionFailed { message: msg, session_id: None }`
- Every pattern match on these variants
- Locations: `src/adapters/subprocess.rs` (4+ sites), `src/application/usecases.rs` (test mocks), `tests/*.rs` (integration test mocks)

### Phase 1: Rig Config — Agent Adapter Selector

Work in `src/domain/value_objects.rs` (`RigAgentConfig`).

**Changes:**

1. **Replace `RigAgentConfig` fields** — remove `cli_path` and `cli_args`, add `agent_adapter`:
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(rename_all = "kebab-case")]
   pub enum AgentAdapter {
       /// Plain text stdout via subprocess (current behaviour).
       PiStdio,
       /// JSON-L stream with metadata extraction.
       PiJson,
   }

   /// Rig-level agent configuration.
   /// Selects which adapter to use — no invocation details.
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   pub struct RigAgentConfig {
       #[serde(default = "default_agent_adapter")]
       pub agent_adapter: AgentAdapter,
   }

   fn default_agent_adapter() -> AgentAdapter {
       AgentAdapter::PiStdio
   }
   ```

2. **YAML config example:**
   ```yaml
   agent-adapter: pi-json
   ```

3. **Each adapter hardcodes its invocation contract** (in the adapter itself, not config):
   - `PiStdioAgentRunner`: binary = `"pi"`, flags from `AgentConfig::build_cli_args()`
   - `PiJsonAgentRunner`: binary = `"pi"`, flags from `AgentConfig::build_cli_args()` + `--mode json`

**Tests (domain):**
- `test_agent_adapter_default_pistdio()` — missing field → `PiStdio`
- `test_agent_adapter_pijson_from_yaml()` — `agent-adapter: pi-json` → `PiJson`
- `test_agent_adapter_pistdio_from_yaml()` — `agent-adapter: pi-stdio` → `PiStdio`
- `test_agent_adapter_invalid_yaml()` — `agent-adapter: unknown` → parse error
- `test_rig_agent_config_serialization_roundtrip()` — full config with `agent_adapter` round-trips
- `test_rig_agent_config_no_cli_path_or_args()` — struct has no `cli_path` or `cli_args` fields

### Phase 2: Adapter — JsonSubprocessAgentRunner

New file: `src/adapters/json_subprocess.rs`.

**Design:**

The JSON adapter is similar to `SubprocessAgentRunner` but:
1. Appends `--mode` and `json` to `cli_args`
2. Reads stdout line-by-line as JSON-L (not as a single string)
3. Parses the first line for session ID (`{"type":"session","id":"..."}`)
4. Parses the `agent_end` line for token usage and final response text
5. Returns `AgentOutput` with populated `metadata`
6. On error, includes captured `session_id` in the `PortError`

**Key implementation detail — session ID capture on timeout:**

The session ID appears in the FIRST line of JSON-L output. Even if the subprocess is killed on timeout, the first line may have already been written to the pipe buffer. The adapter must:
1. Read stdout line-by-line using `BufRead`
2. Parse the session ID from the first line immediately
3. Store it in a variable accessible on the error path
4. If the process is killed, return `PortError::Timeout { session_id, .. }` with the captured ID

**Implementation approach:**

The subprocess stdout is read using `std::io::BufReader` line-by-line. Lines are parsed as `serde_json::Value`. The adapter tracks:
- `session_id: Option<String>` — set from first `type: "session"` line
- `response_text: String` — accumulated from `type: "message_end"` with `role: "assistant"` content
- `token_usage: Option<TokenUsage>` — set from `type: "agent_end"` usage

On successful completion (exit code 0):
```rust
Ok(AgentOutput {
    stdout: response_text,
    stderr,
    exit_code: 0,
    metadata: Some(AgentInvocationMetadata { session_id, token_usage }),
})
```

On timeout (process killed):
```rust
Err(PortError::Timeout {
    message: format!("..."),
    session_id,  // captured from first line
})
```

On non-zero exit:
```rust
Err(PortError::AgentExecutionFailed {
    message: format!("..."),
    session_id,  // captured from first line
})
```

On command not found:
```rust
Err(PortError::CommandNotFound(msg))  // no session_id — process never started
```

**Graceful degradation:**

If JSON-L parsing fails (malformed output from Pi), fall back to treating stdout as plain text:
```rust
Ok(AgentOutput {
    stdout: raw_stdout,  // the raw string
    stderr,
    exit_code,
    metadata: None,  // no metadata available
})
```

**Tests (adapter unit):**

For tests, we need a way to simulate JSON-L output without spawning a real `pi` process. The approach: use `sh -c` to echo known JSON-L lines.

- `test_json_runner_parses_session_id()` — subprocess emits `{"type":"session","id":"abc-123"}` as first line → metadata.session_id = Some("abc-123")
- `test_json_runner_parses_token_usage()` — subprocess emits agent_end with usage → metadata.token_usage populated
- `test_json_runner_parses_response_text()` — subprocess emits message_end with content → stdout contains the response text
- `test_json_runner_timeout_captures_session_id()` — subprocess sleeps (killed on timeout), but first line is session → error.session_id is Some
- `test_json_runner_nonzero_exit_captures_session_id()` — subprocess exits with code 1 after emitting session line → error.session_id is Some
- `test_json_runner_command_not_found()` — missing binary → CommandNotFound, no session_id
- `test_json_runner_malformed_json_fallback()` — garbled output → stdout is raw string, metadata is None
- `test_json_runner_empty_output()` — no output → stdout is empty, metadata is None
- `test_json_runner_adds_mode_json_flag()` — verify `--mode json` is in cli_args

**Test helper:**
```rust
fn json_subprocess_output(session_id: &str, response: &str) -> ExecutionContext {
    // Uses `sh -c 'echo ...'` to emit known JSON-L lines
    let script = format!(
        r#"echo '{{"type":"session","id":"{}"}}'; echo '{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"{}"}}]}}]}}'"#,
        session_id, response
    );
    ExecutionContext {
        cli_path: "sh".to_string(),
        cli_args: vec!["-c".to_string(), script],
        // ... other fields
    }
}
```

### Phase 3: Composition Root — Adapter Selection

Work in `src/server.rs` (composition root), `src/adapters/mod.rs`, and `src/application/usecases.rs` (ProcessStrand).

**Changes:**

1. **Rename existing adapter:** `SubprocessAgentRunner` → `PiStdioAgentRunner` (file: `src/adapters/pi_stdio.rs`). This is a rename only — no behavioural change.

2. **`src/server.rs`** — Select adapter based on `rig_config.agent_adapter`:
   ```rust
   let agent_runner: Arc<dyn AgentRunner> = match config.rig_config.agent_adapter {
       AgentAdapter::PiJson => Arc::new(PiJsonAgentRunner::with_timeout(config.agent_timeout)),
       AgentAdapter::PiStdio => Arc::new(PiStdioAgentRunner::with_timeout(config.agent_timeout)),
   };
   ```

3. **`src/adapters/mod.rs`** — Add `pub mod pi_json;`, rename `subprocess` → `pi_stdio`

4. **`src/lib.rs`** — Re-exports updated to match new module names

5. **`src/application/usecases.rs`** — `ProcessStrand::execute()` no longer constructs `cli_path`/`cli_args` from `RigAgentConfig`. Instead, the selected adapter receives `AgentConfig` (from profile + knot) and builds its own CLI args internally.

**Tests (composition/integration):**
- `test_composition_uses_json_runner()` — with `agent_adapter: pi-json`, runner is PiJsonAgentRunner
- `test_composition_uses_stdio_runner()` — with `agent_adapter: pi-stdio` or default, runner is PiStdioAgentRunner

### Phase 4: Integration Tests and Verification

- [x] Integration test: `test_json_invocation_full_pipeline()` — start Knot with `agent_adapter: pi-json`, trigger strand event with real `pi` binary, verify tie-off contains response text AND metadata was captured (verify via loom-log or state)
- [x] Integration test: `test_stdio_invocation_full_pipeline()` — start Knot with `agent_adapter: pi-stdio` (default), verify existing behaviour unchanged (regression)
- [x] Integration test: `test_json_invocation_timeout_captures_session_id()` — short timeout, verify session_id captured even on failure (check via loom-log or error path)
- [x] Regression: all existing pipeline tests still pass (especially `tests/pipeline.rs`, `tests/profile_timeout.rs`)
- [x] Run `cargo clippy`, fix warnings
- [x] Run full test suite, verify all tests pass

### Phase 5: Domain Glossary

- [x] Update `project/domain-glossary.md`:
  - `Invocation mode` — how Knot communicates with the agent CLI (`stdio` for plain text, `json` for JSON-L with metadata)
  - `Agent invocation metadata` — session ID and token usage captured from Pi's JSON-L output

## Notes

- `SubprocessAgentRunner` is renamed to `PiStdioAgentRunner` (no behavioural change). The existing code path is preserved.
- `RigAgentConfig` loses `cli_path` and `cli_args`. These were never used meaningfully at runtime — the test that set `--verbose` was synthetic. If a need for per-rig extra flags emerges later, it would be added as an explicit field on a specific adapter, not a generic bag.
- The `AgentOutput.metadata` field is `Option` so all existing callers that only read `.stdout` continue to work.
- The PortError shape change (newtype → struct variant for Timeout and AgentExecutionFailed) is a **breaking change** within the crate. All construction sites and pattern matches must be updated. This is Phase 0 work — done first so all subsequent phases can use the new shape.
- The JSON-L parser reads line-by-line. It does NOT buffer the entire output. This is important for timeout detection — if the process hangs, we still have the session_id from the first line.
- Pi's `--mode json` output is newline-delimited JSON. Each line is a complete JSON object. The parser uses `serde_json::from_str::<serde_json::Value>` per line, then pattern-matches on the `type` field.
- This plan does NOT change how the prompt is sent to the agent (still via stdin). It only changes how stdout is interpreted.
- Plan 47 (Session Resume) depends on this plan's `session_id` capture, `AgentInvocationMetadata`, and `PortError::is_resumable()`.
- ADR-009 documents the decision to use agent-specific adapters rather than a generic CLI wrapper.
