# Plan 088: Compaction Assurance — Auto-Compaction Always On, Overflow Recovery Continues the Session

## Related Plans

Rides on, unchanged:

- [079 Context Overflow — Compact and Continue](../079-context-overflow-compact-and-continue/context-overflow-compact-and-continue-plan.md)
  — pi's built-in compact-and-continue is the overflow mechanism:
  threshold auto-compaction at `contextWindow − reserveTokens`, and on
  an overflow error pi removes the error message, compacts, and
  **auto-retries the prompt in-process**
  (`compaction_end { reason: "overflow", willRetry: true }`). Knot's
  part: parse `compaction_end` into `CompactionRecord`, record
  `ContextCompacted` loom + system events per successful compaction,
  and fail fast (`ContextLimitReached`, non-resumable) when the kept
  context still cannot fit.
- [080 Context Overflow Without Compaction — Fail Fast, Warn at
  Startup](../080-overflow-error-fail-fast/overflow-error-fail-fast-plan.md)
  — `effective_pi_compaction_enabled` (project `.pi/settings.json`
  over global `~/.pi/agent/settings.json`, pi default `true`) and the
  startup warning when the effective setting is disabled.
- [084 Graceful Completion](../084-graceful-completion/graceful-completion-plan.md)
  — the `pi-rpc` runner; its output assembly reuses
  `PiJsonAgentRunner::parse_stdout`, so `compaction_end` events flow
  through the RPC stream unchanged and
  `PiJsonAgentRunner::terminal_overflow` is already the RPC runner's
  fail-fast gate.
- [081 Inactivity Timeout](../081-inactivity-timeout/inactivity-timeout-plan.md)
  — the watchdog whose interaction with long summarization calls is
  documented in Notes (no code).

The rig owner's requirement is four guarantees:

1. **Auto-compaction is always on** for rig sessions.
2. **It fires on a context overflow** (compact-and-retry), not just at
   the proactive threshold.
3. **The session continues after compaction** — within the run, and
   across session-resume re-entry.
4. **`compaction_start` and `compaction_end` surface as live events** —
   emitted the moment they are observed in the stream, not after the
   invocation: a long run that compacts several times shows each span
   in the service log as it happens (a live signal of task size and
   that the session is continuing).

## State of the world (mostly done)

| Requirement | Today |
|---|---|
| 1. Always on | knot-init (4.6.0+) **seeds** the project `.pi/settings.json` (`compaction.enabled: true`, never overwriting an existing file); plan 080 **warns** at startup when the effective setting is off. ⚠️ Warn-only: a legacy rig (no project file) with a global `enabled: false` gets a warning and nothing more — overflow then fails strands with `ContextLimitReached` instead of recovering. |
| 2. Fires on overflow | ✅ pi's in-process compact-and-retry (079 verified against pi 0.78.x source). Once per user message; every session-resume (a new user message) gets a fresh recovery chance. |
| 3. Continues after | Within the run: ✅ the stream keeps flowing after `compaction_end`; the session id (RPC: `get_state`; JSON: session header) is captured independently of compaction. Across re-entry: the compaction entry persists in the session file and `--session-id` re-entry loads the post-compaction state — ⚠️ but **untested**: the RPC mock harness has no overflow tests at all (the JSON adapter has both terminal and non-terminal, `tests/adapters.rs`), and no test covers re-entry into a compaction-bearing session. |
| 4. Start/end logged as events | ⚠️ Only **successful** `compaction_end` is logged today — as `ContextCompacted` (loom event + `[KNOT][EVENT]` line + plan 082 system event) — and **post-hoc**: `log_compactions` runs after the invocation returns, so a long run's compactions become visible only at the end. `compaction_start` is ignored entirely in `parse_stdout` (a test asserts it produces no record), and **failed** `compaction_end` records are captured in the metadata but filtered out of logging (plan 079's `failed_compaction_not_logged` — deliberate then: "only successful compactions mark context pressure"). |

This plan closes the four ⚠️ gaps. It adds **no runtime mechanism** —
overflow recovery stays pi's, and Knot's role stays observe + fail
fast.

## Design decisions

**D1 — Startup self-heal of the project settings, never the global.**
The project `.pi/settings.json` is Knot's own file (knot-init seeds
it); the service maintains it at startup. At each rig's startup,
before watcher registration (next to the plan 080 check in
`src/server.rs`):

- project file has an **explicit** `compaction.enabled` (true or
  false) → do nothing; the 080 warning still fires for an explicit
  project-level `false` (an operator's deliberate opt-out stays
  honoured and loud).
- project file **absent, or parseable but without an explicit
  `compaction.enabled`**, and the effective setting resolves to
  disabled (i.e. the global file explicitly says `false`) →
  **merge** `{"compaction": {"enabled": true}}` into the project file,
  preserving every existing key and file. This is the legacy-rig case
  the warning used to cover.
- project file **unparseable** → do **not** clobber unknown content;
  fall back to the 080 warning.
- any write failure (permissions, missing `.pi/` parent that cannot be
  created) → non-fatal; fall back to the 080 warning.

The global `~/.pi/agent/settings.json` is the operator's and is never
written. After D1, "always on" is guaranteed except for an explicit
project-level opt-out (warned) or a broken project file (warned).

**D2 — No new compact trigger.** An active `compact` RPC command
trigger (a `ctx-compact-limit` key) was considered and set aside:
auto-compaction plus overflow compact-and-retry already cover the
requirement, and Knot-triggered compaction would add a config surface
and a second decision point for no coverage gain. The RPC channel
remains available for a future plan if telemetry says otherwise.

**D3 — RPC adapter test parity with the JSON adapter.** The RPC mock
harness gains the overflow scenarios the JSON adapter already has, so
the RPC runner's reuse of `parse_stdout` / `terminal_overflow` is
verified end-to-end through a real (mock) process: compaction events
in the RPC stream, success after recovery, and
`ContextLimitReached` on terminal overflow.

**D4 — Continuity invariant, tested at the Knot boundary.** pi owns
"load the post-compaction session"; Knot owns "re-enter with the
captured session id". The test pins the Knot half: a compaction-bearing
run that needs re-entry (nudge / session-resume) re-enters with
`--session-id <captured id>`, and the compaction is recorded
(`ContextCompacted`) for the attempt that observed it.

**D5 — `compaction_start` and every `compaction_end` become events,
emitted live as the stream produces them.**

- **Loom events** (`src/domain/events.rs`) — three events, one per
  stream event kind:
  - `CompactionStarted { loom_id, knot_id, strand_path, session_id,
    reason, attempt, timestamp }` — the span begins
  - `ContextCompacted { … }` — successful end (**existing event,
    unchanged shape**; it is the operator-facing context-pressure
    signal — troubleshooting: "check `ContextCompacted` entries …
    `reason: "overflow"` = limit hit" — and downstream readers keep
    that contract). Only its *timing* changes: written when the
    `compaction_end` line is observed, not after the invocation.
  - `ContextCompactionFailed { loom_id, knot_id, strand_path,
    session_id, reason, error: String, aborted: bool, attempt,
    timestamp }` — failed/aborted end (carries pi's `errorMessage`).
- **Live observation** — the runners see the stream line-by-line;
  they report span boundaries to an observer callback instead of the
  usecase reading them off the metadata afterwards:
  - `CompactionObservation` enum (`src/application/ports.rs`):
    `Started { session_id: Option<String>, reason: String }` |
    `Ended { session_id: Option<String>, record: CompactionRecord }`.
  - New `AgentRunner` default trait method
    `execute_with_config_and_observer(…, observer:
    Option<Arc<dyn Fn(&CompactionObservation) + Send + Sync>>)`
    mirroring the existing `execute_with_config` (default: ignore the
    observer, delegate to `execute_with_config`). Only the two pi
    runners override it; the observer stays out of
    `ExecutionContext` (which derives `Debug/Clone/PartialEq/Eq` and
    has 22 construction sites — a closure field would break the
    derives and ripple everywhere).
  - **pi-json**: the stdout reader gains an optional per-complete-line
    callback (lines are still buffered for the final parse as today).
    A shared helper (`observe_compaction_line`) cheaply prefix-filters
    (`"session"` / `"compaction_start"` / `"compaction_end"`), parses
    the match, captures the session id from the early `session`
    header line into a small shared state (compaction lines always
    follow it), and invokes the observer. `pi-stdio` never observes
    (no JSON stream — same as today).
  - **pi-rpc**: the driver thread already owns the line channel; the
    same helper runs in its loop (session id comes from the
    `get_state` response, already captured).
  - **Multiple spans per session** are the normal case: each
    `compaction_start`/`compaction_end` pair fires its own
    observation in stream order — threshold compactions repeat as the
    context refills, and overflow compact-and-retry fires per user
    message (each nudge/re-entry is a new user message). No pairing
    assumption between starts and ends.
- **Usecase side** (`session_resume.rs`): a per-attempt closure
  (captures `attempt`, loom/knot/strand, `Arc`-cloned loom log +
  emitter) maps each observation to its `LoomEvent` and appends it —
  `append` writes the `[KNOT][EVENT]` line (live in `knot-service.log`)
  and stores it in `RunActivity` — then emits the plan 082 system
  event, following the existing `ContextCompacted` pattern. Failed
  ends route to `ContextCompactionFailed` (the `error.is_none()`
  filter becomes a routing decision). Observer failures are
  best-effort no-ops (matching the steer's send-failure posture).
  Thread-safety: `RunActivity` is a `Mutex` behind `Arc`,
  `InMemoryLoomLog::append` takes `&self`, the emitter is
  `Arc`-based, and `eprintln!` takes the global stderr lock — all
  safe from the reader/driver thread; the closure holds no locks
  across the `append` call.
- **The post-hoc `log_compactions` is removed** (superseded by the
  live path for both pi runners); `TestAgentRunner` invokes the
  observer with its mock's compaction records (a `Started` before
  each `Ended`) so usecase-level tests keep parity.
- **Metadata records stay** — `compactions: Vec<CompactionRecord>`
  (now with `aborted: bool`, serde default) and a new
  `compaction_starts: Vec<String>` (start reasons, stream order) are
  still captured by the shared `parse_stdout`: `terminal_overflow`
  runs on the end records after the final parse, and the records
  remain useful for tests and post-hoc debugging. They are no longer
  the *source* of the events.

## Phases

### Phase 0: Failing Tests

1. `src/server.rs` — startup self-heal (unit tests on the new
   `ensure_pi_compaction_enabled`-style helper, tempdir fixtures for
   project + global settings):
   - `self_heal_creates_project_file_when_global_disabled` — global
     `{"compaction":{"enabled":false}}`, no project file → project
     file created with `compaction.enabled == true`.
   - `self_heal_merges_preserving_existing_keys` — project file with
     unrelated keys (e.g. `{"theme":"dark"}`), no `compaction` key,
     global disabled → file gains `compaction.enabled` and keeps
     `theme`.
   - `self_heal_preserves_existing_compaction_keys` — project file
     `{"compaction":{"reserveTokens":8192}}` → gains `enabled: true`,
     keeps `reserveTokens`.
   - `self_heal_respects_explicit_project_false` — project file
     `{"compaction":{"enabled":false}}` → file untouched.
   - `self_heal_noop_when_effective_enabled` — global disabled,
     project `{"compaction":{"enabled":true}}` → untouched; and no
     global (pi default on) → untouched.
   - `self_heal_refuses_unparseable_project_file` — project file with
     broken JSON → untouched, warning path taken.
2. `src/adapters/pi_rpc.rs` — overflow parity (extend
   `rpc_mock_script` to emit `compaction_end` lines in the stream):
   - `rpc_overflow_recovered_is_success` — stream: `get_state`
     response, `compaction_end { overflow, willRetry: true, result
     { tokensBefore: 150000 } }`, `agent_end` (stop, final text) →
     `Ok`, `metadata.compactions` carries the record, response text
     intact.
   - `rpc_terminal_overflow_returns_context_limit_reached` — stream:
     `compaction_end { overflow, willRetry: true, … }` followed by
     `compaction_end { overflow, willRetry: false, errorMessage }`,
     `agent_end` (error message only) →
     `Err(PortError::ContextLimitReached)`, session id carried from
     `get_state`.
   - `rpc_compaction_end_reasons_recorded` — a `threshold` (or
     `manual`) `compaction_end` mid-stream is recorded with its
     reason; a `compaction_start` line produces a start record (D5).
3. `src/application/session_resume.rs` —
   - `compaction_then_reentry_uses_captured_session_id` — attempt 1
     mock output: `Ok` with a compaction record and empty stdout
     (nudge case) → attempt 2's context carries
     `--session-id <captured id>` in `extra_args`, and the attempt 1
     compaction was logged `ContextCompacted` (reason preserved).
4. `src/adapters/pi_json.rs` / `src/adapters/service_log.rs` /
   `src/application/session_resume.rs` — live event emission (D5):
   - `pi_json.rs` observer tests (recording observer —
     `Arc<Mutex<Vec<CompactionObservation>>>`):
     `observer_receives_start_and_end_in_stream_order` — mock stream
     with `session` header, `compaction_start { reason: "threshold" }`,
     `compaction_end { threshold, result { tokensBefore } }` →
     `Started { session_id: Some("sess-…"), reason: "threshold" }`
     then `Ended { record { error: None, … } }`, in order;
     `observer_sees_multiple_spans` — two compaction spans in one
     stream → four observations, in stream order;
     `observer_receives_failed_end` — `compaction_end { overflow,
     willRetry: false, errorMessage }` → `Ended` with
     `record.error: Some(…)`; `observer_none_is_noop` — no observer
     attached, compaction in stream → run succeeds, no panics.
   - `pi_rpc.rs` — `rpc_observer_receives_compaction_events` — the
     RPC mock stream emits the same span; the driver's line path
     delivers the observations (session id from `get_state`).
   - `service_log.rs` — render tests for both new variants, mirroring
     the existing `ContextCompacted` line tests
     (`CompactionStarted loom=… knot=… strand=… session=… reason=…
     attempt=…`; `ContextCompactionFailed … reason=… error=…
     aborted=… attempt=…`).
   - `session_resume.rs` (observer-driven, replaces the post-hoc
     `log_compactions` assertions) — `compaction_start_logged` (start
     + successful end → `CompactionStarted` then `ContextCompacted` in
     the loom log, in order, with the attempt); `failed_compaction_logged_as_failed_event`
     (**rewrites** plan 079's `failed_compaction_not_logged`: a failed
     end now produces `ContextCompactionFailed` carrying the error
     text, and still no `ContextCompacted`); `aborted_compaction_logged`
     (aborted end → `ContextCompactionFailed` with `aborted: true`).
   - `process_strand.rs` — extend the existing compaction integration
     test (the loom-log `ContextCompacted { attempt: 1 }` case) to
     assert `CompactionStarted` precedes it in the loom log.
   - `pi_json.rs` parser (metadata) tests: `compaction_start_recorded_
     with_reason` — `compaction_starts == ["threshold"]` (replaces
     `test_json_runner_ignores_compaction_start`); `compaction_end_
     aborted_captured` — record with `aborted: true`, `error: None`.

### Phase 1: Startup Self-Heal (D1)

`src/server.rs`, beside `warn_if_pi_compaction_disabled`:

1. Extract the project-root derivation (rig parent → CWD → rig dir)
   into a shared helper used by both the self-heal and the 080
   warning.
2. New `ensure_pi_compaction_enabled(rig_dir)` implementing the D1
   table; reuses `PiJsonAgentRunner::read_pi_compaction_enabled` /
   `effective_pi_compaction_enabled`; merge-write via
   `serde_json::Value` (read → set key → pretty-write), creating
   `.pi/` as needed.
3. Keep `warn_if_pi_compaction_disabled` as the fallback for every
   "did not / could not enable" branch — the log message gains the
   reason (explicit opt-out / unparseable file / write failed).
4. Call it in the startup sequence at the current 080 call site
   (before watcher registration; per-rig).

### Phase 2: RPC Adapter Overflow Test Parity (D3)

1. `rpc_mock_script` gains compaction-stream variants (emit
   `compaction_start` / `compaction_end` lines before `agent_end`;
   the `*compact*)` stdin case is **not** needed — D2).
2. Implement until the Phase 0 RPC tests pass; no production code
   change is expected (the path exists via `parse_stdout` /
   `terminal_overflow`) — if a test fails, the fix is in the RPC
   runner, and the failure is the plan working as intended.

### Phase 3: Continuity After Compaction (D4)

Implement until `compaction_then_reentry_uses_captured_session_id`
passes; expected to be test-only (re-entry already appends
`--session-id` from the captured id regardless of compactions).

### Phase 4: Live Compaction Event Emission (D5)

1. `src/application/ports.rs` — `CompactionObservation`, the
   `execute_with_config_and_observer` default trait method,
   `CompactionRecord.aborted` (serde default),
   `AgentInvocationMetadata.compaction_starts: Vec<String>` (serde
   default); doc comments updated (plan 079 → 088).
2. `src/adapters/pi_json.rs` — `observe_compaction_line` helper
   (prefix filter → parse → session-id capture → observer) and the
   reader's per-complete-line callback; `parse_stdout` captures
   `compaction_start.reason` and `compaction_end.aborted` into the
   metadata; `execute_with_config_and_observer` override (internal
   `execute_inner(ctx, observer)` split so `execute` stays unchanged).
3. `src/adapters/pi_rpc.rs` — the driver's line loop calls the same
   helper (session id from the already-captured `get_state` response);
   the `execute_with_config_and_observer` override.
4. `src/domain/events.rs` — `CompactionStarted` and
   `ContextCompactionFailed` variants with doc comments.
5. `src/adapters/service_log.rs` — render arms for both variants.
6. `src/application/session_resume.rs` — the per-attempt observer
   closure (loom event + `[KNOT][EVENT]` line + plan 082 system
   event, `attempt` captured); **remove** `log_compactions` and its
   call sites; `TestAgentRunner` fires the observer with its mock's
   compaction records.
7. Rewrite the two superseded tests (`test_json_runner_ignores_
   compaction_start`, `failed_compaction_not_logged`) and implement
   until the Phase 0 live-emission tests pass.

### Phase 5: Verify + Regression

- `cargo test` — full suite green; plan 079/080 JSON-adapter overflow
  tests and the plan 084/086 steer/water-mark tests pass (the two
  superseded tests rewritten per Phase 4.7, not deleted silently).
- `cargo clippy --all-targets` — no new warnings.
- **Thread-safety spot-check** — the observer runs on the
  reader/driver thread: confirm no lock is held across
  `loom_log.append` (the `RunActivity` mutex is taken only inside it)
  and no path deadlocks the `join_all` drain (the closure is the
  reader thread's own work, not a join target).
- Manual spot-check (allowed — no live rig runs): start the service
  against a tempdir rig with a global-disabled fixture and confirm the
  project file is seeded and the startup log shows the enablement.
- No live rig runs in this repository (AGENTS.md): verification is
  `cargo test` / `cargo clippy` + the mock-CLI harness.

### Phase 6: Docs + Version

1. `docs/release-notes.md` — v0.45.0: startup self-heal of
   `compaction.enabled` (merge, never clobber, global untouched,
   explicit opt-out honoured + warned), RPC overflow test parity,
   continuity test, and **live** compaction events —
   `CompactionStarted` / `ContextCompacted` / `ContextCompactionFailed`
   are written as the stream produces them (a long run that compacts
   several times shows each span in `knot-service.log` as it happens);
   `ContextCompacted` is unchanged in shape and still success-only.
2. `docs/concepts.md` — state the overflow story as the default:
   (1) pi auto-compaction at the threshold, (2) overflow
   compact-and-retry in-process, (3) terminal-overflow fail-fast; the
   water-mark wrap-up (084/086) is an **optional** add-on, disabled by
   default (no `ctx-wrap-up-limit` ⇒ off).
3. `docs/troubleshooting.md` — the context-limit entry: compaction is
   now self-healed at startup; the compaction events
   (`CompactionStarted` / `ContextCompacted` / `ContextCompactionFailed`)
   appear in the service log **as they happen** — a repeated
   `CompactionStarted` stream is the live signal that a task is large
   enough to fill the context more than once; reasons
   (`threshold` / `overflow` / `manual`); a failed end is now visible
   instead of silent; a remaining startup warning means an explicit
   project opt-out or an unparseable settings file.
4. `knot-update` skill — changelog entry: no document-format change
   (behaviour-only hardening); knot-init's seed is now backed by
   service self-heal.
5. Version bump 0.44.1 → 0.45.0; `cargo install --path .`.

## Notes

- **Watchdog vs. long summarization (documented edge, no code in v1).**
  Between `compaction_start` and `compaction_end` no stream bytes
  flow while the summarization LLM call runs; a summarization longer
  than the inactivity window (default 300 s, plan 081) could trip the
  watchdog mid-compaction. With the default 20k-token keep-recent
  budget a summarization call is far shorter than that; if it is ever
  observed, the remedy is a compaction-aware activity tick (the
  `compaction_start` line already refreshes activity on arrival).
- **Alternative considered and set aside (D2):** an active `compact`
  RPC trigger keyed to a new `ctx-compact-limit` alias setting. Not
  needed for the four guarantees; revisit only if sessions are seen
  reaching the overflow path (failed LLM call + retry) when a
  Knot-scheduled compact would have avoided it.
- **pi-json / pi-stdio unchanged** — the overflow story is identical
  (same `compaction_end` stream, same fail-fast); only the RPC parity
  tests and the shared startup self-heal touch them.
