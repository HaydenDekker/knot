# Plan 081: Inactivity Timeout — Kill Blocked Sessions, Restart with a Blocking-Call Note

## Related Plans

Builds on `rig-log-notification-and-timeout.md` (the existing
**total** wall-clock timeout, rig-log `TimeoutExceeded`, per-profile
`timeout`), `session-resume-on-invocation-failure.md` (the retry loop,
`--session-id` re-entry, budget tracking), [077 Empty Response Is Not a
Timeout](../077-empty-response-not-timeout/empty-response-not-timeout-plan.md)
(cause-accurate error classification) and
[078 Final-Response Request](../078-final-response-request/final-response-request-plan.md)
(the nudge-prompt convention: one greppable const per cause-specific
retry note).

## Problem

Knot's only timeout today is a **total wall-clock timeout**: the adapter
spawns a kill-thread that SIGKILLs the pi process after the effective
deadline (runner default 300s, or the profile `timeout`). That measures
*total work*, not *stall*. Two failure shapes fall through:

1. **Blocked external call.** The agent runs a bash command (or calls an
   external service) that hangs — deadlocked process, command waiting on
   stdin, stalled network call. The pi session is alive but **silent**:
   no thinking, no response, no output. If the profile has a large
   timeout (e.g. 3600s for a thorough review), the rig sits silent for
   tens of minutes before the total timeout fires — and the restart it
   triggers carries the generic plan-078 nudge
   ("please produce your final response"), which does not tell the agent
   *why* it was stopped or how to avoid the same block.
2. **Healthy long sessions die.** The inverse: an active session doing
   many tool calls (each quick) with a 300s total budget is killed even
   though nothing is stuck. The total timeout conflates "work" with
   "stall".

Spec: a rig-global `inactivity-timeout-seconds` (default **300**). When
the session produces **no output** for that window, Knot stops the
session and restarts it with a note:

> The last call blocked for more than xx seconds. If you have a
> long-running task, ensure it's emitting a progress update at least
> once within the x-minute window.

## Why byte-level silence is a faithful stall detector

pi's `--mode json` stream emits a line for every unit of activity
(`session`, `agent_start`, `turn_start`, `message_start`,
`message_update` streaming deltas, `tool_execution_start`,
`tool_execution_update`, `tool_execution_end`, `message_end`,
`turn_end`, `agent_end`, `compaction_*`, `auto_retry_*` — see pi's
`docs/json.md`). Critically, pi's bash tool **streams command output**:
`dist/core/tools/bash.js` calls `onUpdate` throttled at
`BASH_UPDATE_THROTTLE_MS = 100`, and `agent-session.js` relays those as
`tool_execution_update` lines on stdout.

Therefore:

- A **healthy** long-running command (`cargo build`, a test suite)
  keeps emitting `tool_execution_update` lines → never silent beyond a
  few hundred milliseconds → the inactivity timer keeps resetting.
- A **hung** command (or a stalled provider call, or a deadlocked
  process) emits nothing → the stream goes quiet → the timer fires.
- The last line before the silence is typically
  `tool_execution_start` (carrying `toolName` + `args`) → the adapter
  can name the blocked call in the restart note.

**Detection is byte-level** — any byte on the child's stdout/stderr
resets the timer. No JSON parsing is involved in the timer itself;
parsing happens only *after* a kill, to extract the session ID and the
blocked call from the already-accumulated buffer.

Caveat (documented, not blocking): under the `pi-stdio` adapter there is
no structured stream and no session ID. Byte-level detection still works
(print mode streams assistant text), but tool output does not reach pi's
stdout until the tool completes, so even a healthy long tool call can
exceed the window, and the restart is a **fresh** session (no
`--session-id`). The feature is most effective under `pi-json` —
which this plan makes the default (Phase 1).

## Target — behavioural contract

### Configuration (rig-global)

`rig/.workspace-agent-config.yaml`:

```yaml
agent-adapter: pi-json
inactivity-timeout-seconds: 300   # default when absent; 0 disables
```

- `RigAgentConfig` gains `inactivity_timeout_secs: u64` with
  `#[serde(default = "default_inactivity_timeout_secs")]` → **300** when
  the key is absent (old config files keep working — no migration).
- Explicit `0` disables the watchdog. `> 0` is the window in seconds.
- Loaded at startup with the rest of the rig config (no hot reload —
  restart Knot after editing the file), and passed into the agent runner
  at the composition root, exactly like `agent-adapter` today.
- Per-profile / per-knot override: **out of scope** (follow-up;
  `ExecutionContext.timeout` already shows the per-call plumbing).

### Watchdog (adapters — `pi-json` and `pi-stdio`)

The current `wait_with_output()` pattern buffers all output until exit —
it cannot observe liveness. Replace with:

- **stdout reader thread** — chunk-reads into a shared accumulated
  buffer (`Arc<Mutex<Vec<u8>>>`) and updates
  `last_activity: Arc<AtomicU64>` (unix nanos) on every non-empty read.
  Initialised at spawn, so the pre-first-byte window counts (a provider
  that cannot answer the first token within the window is exactly what
  we want flagged).
- **stderr reader thread** — same, into its own buffer (stderr still
  feeds the existing error messages). Stderr bytes also reset the timer
  (a process writing diagnostics is alive; being lenient here avoids
  spurious kills).
- **watchdog thread** — polls every 250 ms against both deadlines:
  - `now − last_activity > inactivity_timeout` → SIGKILL the process
    group (child + subprocesses, same `kill(-pgid, SIGKILL)` as today),
    record kill reason `Inactivity`;
  - `start.elapsed() > effective_total_timeout` → SIGKILL, record
    reason `Total` (**unchanged existing behaviour** — the total
    wall-clock timeout is not modified by this plan: same default
    300s, same per-profile `timeout` override, same remaining-budget
    math in the retry loop, same `PortError::Timeout` outcome).
- **Precedence when both elapse** (e.g. a fully silent session with
  equal windows): the watchdog checks inactivity first and it wins —
  it is the more specific diagnosis and carries the restart note. A
  session that keeps producing output is bounded by the total timeout
  exactly as today; if the inactivity window is larger than the
  remaining total budget (or disabled), the total fires first, as
  today.
- **Classification on exit** (main thread, after joining reader + wait
  threads — the existing 2×-deadline join guard is preserved):
  - `status.code().is_some()` → the child exited on its own: normal
    success / non-zero paths, kill reason ignored (the child won the
    race — same intent as today's `cancelled` flag);
  - killed + `Total` → `PortError::Timeout` (existing shape; session ID
    parsed from the accumulated stdout as today);
  - killed + `Inactivity` → parse accumulated stdout for `session_id`
    (pi-json) and the blocked call → `PortError::AgentInactivity`.
- The existing warning-line / `cancelled`-flag behaviour is kept; the
  warning names the reason (`inactivity` vs `timeout`).

### Error

New variant in `src/application/ports.rs`:

```rust
/// The agent session produced no output for the inactivity window —
/// killed by the watchdog, most likely a blocked tool call or stalled
/// provider. Resumable; unlike `Timeout` it is resumable **without** a
/// session ID (a fresh restart with the blocking-call note is
/// meaningful — knots are idempotent).
AgentInactivity {
    /// Human-readable description (Display / loom-log / rig-log text).
    message: String,
    /// How long the session was silent (seconds).
    silent_secs: u64,
    /// The configured inactivity window (seconds).
    window_secs: u64,
    /// The blocked call, when derivable from the stream
    /// (e.g. `bash("npm run build")`).
    blocked_call: Option<String>,
    session_id: Option<String>,
}
```

- `Display` → `inactivity: {message}` (greppable, mirrors the
  `timeout:` / `no final response:` prefixes).
- `session_id()` → returns the field (add to the match).
- `is_resumable()` → `true`.
- **Resumability gate change** (the one deliberate exception):
  `execute_with_resume_internal`'s first-attempt gate
  (`!err.is_resumable() || error_session_id.is_none() → give up`)
  treats `AgentInactivity` as retryable even with `session_id == None`
  (pre-session stall, stdio adapter). All other errors keep the
  session-ID requirement.

### Restart with the note (`session_resume.rs`)

On `AgentInactivity` the existing retry loop re-enters:

- **with `--session-id <id>`** when a session ID was captured (pi-json
  — the usual case): the agent resumes the same conversation, sees its
  own interrupted history, and continues;
- **fresh** (no `--session-id`) when it wasn't: the full original
  prompt + strand reference is re-sent (the loop already re-sends the
  prompt each attempt); idempotency makes this safe.

The retry prompt appends a **cause-specific note** instead of the
generic `FINAL_RESPONSE_REQUEST` for the attempt that follows an
inactivity kill:

```
Your last call blocked for more than {silent_secs} seconds with no
output, so your previous turn was stopped. If you have a long-running
task, ensure it emits a progress update at least once within the
{window_secs}-second window (e.g. run it in the background and poll its
output, or stream the output). Continue from where you left off and
produce your final response when done.
```

- `INACTIVITY_RESTART_NOTE` — built via `format!` from the error's
  `silent_secs` / `window_secs` (xx = actual silence; x = the
  configured window; minutes phrasing is derived, e.g. 300s → "5-minute
  window"). Kept as a greppable template const like
  `FINAL_RESPONSE_REQUEST` (user-facing agent text).
- A `pending_note: Option<String>` is set when a failure is classified
  and consumed by the next attempt; non-inactivity failures keep the
  078 text. The prompt still accumulates per-attempt notes as today.
- **Bounds unchanged**: `MAX_RETRIES = 10`, profile timeout budget,
  `MIN_REMAINING_SECS = 5`, `RETRY_DELAY = 10s`
  (`KNOT_RETRY_DELAY_MS` in tests). The inactivity window is
  **per-attempt** and is *not* reduced by the elapsed budget (a stall is
  a stall); if the window exceeds the remaining total budget, the total
  timeout simply fires first.
- **Terminal on exhaustion** (all attempts inactivity): the final
  `match &first_error` gains an arm —
  `PortError::AgentInactivity { .. }` → terminal
  `PortError::AgentInactivity` with
  `"session resume exhausted {MAX_RETRIES} retries after N
  inactivity kills"` (cause-accurate, per 077/078).

### Outcome + observability

- `TieOffOutcome::derive`: `Err(AgentInactivity)` →
  **`TimeoutSkipped`** — no tie-off section written (the tie-off is
  agent output; a stalled session produced no final response), rig-log
  recorded. Same outcome family as a total timeout: a deadline did fire.
- **New loom event** (per stall, mirroring `KnotEmptyResponse`):

  ```rust
  /// The agent session produced no output for the inactivity window and
  /// was killed; the session is being restarted with the
  /// blocking-call note.
  AgentInactivity {
      loom_id: LoomId,
      knot_id: KnotId,
      strand_path: StrandPath,
      session_id: String,            // "" when none was captured
      silent_secs: u64,
      window_secs: u64,
      blocked_call: Option<String>,
      attempt: u32,                  // 1 = first attempt, 2 = first retry…
      timestamp: String,
  }
  ```

- **Rig-log: no new event in v1.** Exhaustion flows through the
  existing `TimeoutSkipped` → `TimeoutExceeded` rig-log path; the
  `error` string carries the inactivity cause. (Alternative considered
  and deferred — see Notes.)
- Loom-log story, successful case:
  `KnotProcessing → AgentInactivity(1) → SessionResumed(1) →
  KnotCompleted → StrandProcessed`; exhausted case:
  `…AgentInactivity(n) → SessionResumed(n) … → KnotFailed →
  StrandProcessed(error)` with a `TimeoutExceeded` rig-log entry and no
  tie-off write.

### `pi-json` helper: name the blocked call

`parse_blocked_call(raw_stdout) -> Option<String>` — scan the
accumulated lines for `tool_execution_start` / `tool_execution_end`;
return the most recent start with no matching end as
`{toolName}({args preview})` (args preview: the `command` field when
present, else the first 80 chars of the args JSON). Best-effort —
`None` is fine (the note works without it).

## Non-Goals

- No per-profile / per-knot inactivity override (rig-global is the spec).
- No changes to pi — the stream already carries everything needed.
- No hot reload of `.workspace-agent-config.yaml`.
- No new HTTP surface (the loom-log / rig-log already expose this).
- No change to total-timeout semantics or to the retry *bounds*.
- No `knot-update` migration entry (additive config key; old files
  parse with the default).

## Phases

**All phases complete 2026-09-03 — released in Knot v0.40.0.**
Phases 0–5 landed as committed; phase 6 (verify + docs + version)
closed the plan: full `cargo test` green (948 lib + integration,
0 failures), no new clippy warnings from this plan, empirical pi
verification (see Implementation Status), docs
(`rig-structure`, `concepts`, `troubleshooting`, release notes),
`knot-init` 4.8.0 (seeds `pi-json` for fresh rigs), `Cargo.toml`
0.39.0 → 0.40.0. `knot-update` gains **no migration entry**
(additive config key — decision in the Phases section below).

### Phase 0: Failing tests

1. `src/adapters/pi_json.rs` (mock CLI scripts via `KNOT_TEST_CLI_PATH`,
   same harness as the existing timeout tests):
   - `execute_inactivity_kill` — script: emit
     `{"type":"session","id":"sess-inact"}`, then `sleep 300`;
     inactivity=200ms, total=30s → `Err(AgentInactivity)`,
     `session_id == "sess-inact"`, `silent_secs` ≥ window, message
     contains `no output for`.
   - `execute_inactivity_disabled` — same script, inactivity `None` →
     **not** `AgentInactivity` (falls to the total timeout path with a
     small total, or success).
   - `execute_inactivity_reset_by_output` — script: emit a line every
     100ms for ~2s, then `exit 0`; inactivity=200ms → `Ok` (timer kept
     resetting).
   - `execute_inactivity_reset_then_stall` — script: emit one line,
     then `sleep 300` → `AgentInactivity` (first byte resets, then
     stall — proves the reset is not just spawn-time).
   - `execute_inactivity_names_blocked_tool` — script: session line +
     `{"type":"tool_execution_start","toolCallId":"t1","toolName":"bash","args":{"command":"npm run build"}}`,
     then `sleep 300` → message contains `bash` and `npm run build`.
   - `execute_inactivity_before_session_line` — script: immediate
     `sleep 300`; inactivity=200ms → `AgentInactivity {
     session_id: None }`.
   - `execute_total_timeout_still_timeout` — regression: silent script,
     inactivity disabled, small total → `PortError::Timeout` (the two
     watchdogs are distinct).
2. `src/adapters/pi_stdio.rs` — mirror: `execute_inactivity_kill`
   (silent `sleep 300` script → `AgentInactivity { session_id: None }`)
   and `execute_inactivity_reset_by_output` (echoing lines → success).
3. `src/application/session_resume.rs` (`TestAgentRunner`):
   - `inactivity_retry_reenters_session_with_note` — sequence
     `[Err(AgentInactivity{ sid }), Ok("done", sid)]` → `Ok`; runner
     called twice; context[1] `extra_args` contain `--session-id` + sid;
     context[1] `prompt` contains the inactivity note (with the secs
     values) and **not** the bare 078 text; loom-log:
     `AgentInactivity { attempt: 1 }`, `SessionResumed { attempt: 1 }`.
   - `inactivity_retry_fresh_without_session` — sequence
     `[Err(AgentInactivity{ sid: None }), Ok("done")]` → `Ok`;
     context[1] has **no** `--session-id`; prompt contains the note.
     (Covers the new gate: resumable without a session ID.)
   - `inactivity_note_replaces_final_response_request` — after a plain
     `Err(Timeout{ sid })` the retry prompt still contains
     `FINAL_RESPONSE_REQUEST` (078 regression).
   - `inactivity_exhausted_terminal` — sequence
     `[Err(AgentInactivity{ sid }); ×11]`, profile timeout `None`
     (no budget), zero delay → terminal `Err(PortError::AgentInactivity)`
     with message containing `exhausted`; loom-log: 11 ×
     `AgentInactivity` + 10 × `SessionResumed`; runner called 11 times.
4. `src/application/usecases/process_strand.rs`:
   - `process_strand_inactivity_resumed_success` — sequence
     `[Err(AgentInactivity{ sid }), Ok("final", sid)]` (zero retry
     delay); expect: tie-off appended **`Produced`** with `final`;
     loom-log `KnotProcessing`, `AgentInactivity`, `SessionResumed`,
     `KnotCompleted`, `StrandProcessed`; rig-log **empty**; no
     `KnotFailed`.
   - `process_strand_inactivity_exhausted_preserves_tieoff` — all
     attempts inactivity; expect: **no** tie-off write
     (`TimeoutSkipped`); rig-log `TimeoutExceeded` whose `error`
     contains the inactivity message; loom-log `KnotFailed` +
     `StrandProcessed { error: Some(..) }`.
5. `tests/session_resume.rs` integration (mocked ports,
   `KNOT_RETRY_DELAY_MS=0`):
   - `test_inactivity_restarts_with_note` — tie-off `Produced` after
     inactivity→note→success; assert `--session-id` on the second
     invocation and the note text in the captured prompt.
   - `test_inactivity_exhausted_preserves_tieoff` — no new tie-off
     section appended (prior content intact); rig-log has
     `TimeoutExceeded`; queued event removed (late-removal invariant).

### Phase 1: Config

`src/domain/value_objects.rs`:

- `pub const DEFAULT_INACTIVITY_TIMEOUT_SECS: u64 = 300;`
- `RigAgentConfig.inactivity_timeout_secs: u64`
  (`#[serde(default = "default_inactivity_timeout_secs")]`); update
  `default_config()` and the `Eq`/derive set.
- Helper `pub fn inactivity_timeout(&self) -> Option<Duration>` —
  `None` when the value is `0`, else `Duration::from_secs(v)`.
- **Default adapter flip to `pi-json`** (confirmed intent — the
  inactivity feature targets `pi-json`; today the *compiled* default
  is `pi-stdio`): `default_agent_adapter()` and
  `default_config()` → `AgentAdapter::PiJson`. **Existing rigs are
  unaffected** — an explicit `agent-adapter` in
  `.workspace-agent-config.yaml` (including this repo's dev rig,
  which pins `pi-stdio`) wins over the default; the flip only changes
  what fresh rigs get.
- Tests: YAML round-trips (absent → 300; explicit → value; `0` →
  disabled), JSON round-trip, `inactivity_timeout()` mapping,
  `default_config().agent_adapter == PiJson`.

### Phase 2: Watchdog in the adapters

`src/adapters/pi_json.rs` and `src/adapters/pi_stdio.rs` (shared shape —
extract a small `LiveOutput` helper if it reduces duplication between
the two):

- struct field `inactivity_timeout: Option<Duration>`; existing
  constructors set `None`; add one composition-root constructor taking
  `(cli_path, total_timeout, inactivity_timeout)`. The total-timeout
  code path (deadline check, kill, `PortError::Timeout`, warning line)
  is preserved verbatim — the refactor only moves *output capture*
  from `wait_with_output` to the reader threads.
- Replace the `wait_with_output`-on-a-thread capture with the reader +
  watchdog threads per the Target section. Keep:
  process-group spawn (`setpgid`), `kill(-pgid, SIGKILL)`, the 2×
  deadline join guard, and the `cancelled`-style suppression of the
  warning when the child exited first.
- `pi-json` only: `parse_blocked_call()` per the Target section + unit
  tests (no end event → named; matched end → `None`; multiple starts →
  the last open one; malformed lines ignored).
- Error construction: `message` built as
  `no output for {silent_secs}s (inactivity window {window_secs}s){,
  last call: {blocked_call}} ({cli_path}, strand: {strand})`.

### Phase 3: Error + outcome + event

- `src/application/ports.rs`: `PortError::AgentInactivity { … }` +
  `Display` (`inactivity: …`) + `session_id()` arm +
  `is_resumable()` arm; unit tests mirroring the
  `agent_no_response_is_resumable` / display tests.
- `src/domain/entities.rs`: `TieOffOutcome::derive` arm
  `Err(AgentInactivity)` → `TimeoutSkipped`; test.
- `src/domain/events.rs`: `LoomEvent::AgentInactivity { … }` + serde
  round-trip test (field order stable — it is a new variant, no
  compatibility concern).

### Phase 4: Resume loop

`src/application/session_resume.rs`:

- `INACTIVITY_RESTART_NOTE` template const + `format!` helper
  (minutes phrasing: `if window_secs % 60 == 0 → "{window_secs/60}-minute
  window" else → "{window_secs}-second window"`).
- `pending_note: Option<String>` — set on inactivity failure (first
  attempt and in-loop), consumed when building the retry prompt; other
  causes → `FINAL_RESPONSE_REQUEST`.
- Append `LoomEvent::AgentInactivity { attempt, … }` on each inactivity
  failure (attempt 1 on the first-attempt path; `attempt + 1` in-loop,
  same convention as `KnotEmptyResponse`).
- First-attempt gate: `AgentInactivity` bypasses the
  `session_id.is_some()` requirement (fall through to the loop;
  `session_id` stays `None` → the loop's existing
  `if let Some(sid) = session_id` skips `--session-id`).
- Terminal `match &first_error`: add
  `PortError::AgentInactivity { .. }` → terminal `AgentInactivity`
  (exhaustion message per the Target section), ahead of the `_` arm.
- `process_strand.rs`: no code change expected —
  `outcome.is_timeout()` already routes `TimeoutSkipped` to the rig-log
  `TimeoutExceeded` append and skips the tie-off; verify the message
  text carries the inactivity cause (it is `err.to_string()`).

### Phase 5: Wiring

`src/server.rs`, `build_app_context`: pass
`rig_config.inactivity_timeout()` into the new runner constructor for
both adapters (the `cli_path` and non-`cli_path` branches). No
`AppConfig` change — the value flows from `RigAgentConfig`, which is
already loaded at startup. (The default-adapter flip lives in Phase 1
— it is a `RigAgentConfig` change.)

### Phase 6: Verify + docs + version

- `cargo test` full suite — watch the adapter timeout regressions
  (`execute_timeout*`, the wait-thread refactor touches the hot path),
  `tests/pipeline.rs`, `tests/tie_off.rs`, `tests/session_resume.rs`,
  `tests/agent_integration.rs`.
- `cargo clippy --all-targets` clean.
- **Empirical pi check** (real binary, one-off):
  - `pi --mode json -p "run: sleep 400"`-style strand with
    inactivity=60s → killed, restart note, loom-log
    `AgentInactivity` → `SessionResumed`;
  - a healthy outputting command (e.g. `seq 1 100000 | while read n; do
    echo $n; sleep 0.1; done`) → `tool_execution_update` lines keep the
    session alive (confirms the 100ms throttle reaches stdout in json
    mode).
- `docs/configuration/rig-structure.md` — document
  `inactivity-timeout-seconds` (default 300, `0` disables, restart
  required) and update the adapter table for the default flip
  (pi-json: **default** — session IDs + token usage + inactivity
  restart with blocked-call identification; pi-stdio: available,
  inactivity works but restarts are fresh).
- `.agents/skills/knot-init/SKILL.md` — step 5 seeds
  `agent-adapter: pi-json` in `.workspace-agent-config.yaml` (fresh
  rigs get the full inactivity experience); deploy the updated
  sub-skill to `~/.agents/skills-library/knot-init/` and verify with
  diff, per AGENTS.md.
- `docs/concepts.md` — session-resume paragraph: the inactivity
  watchdog, the two timers (silence vs budget), the restart note.
- `docs/troubleshooting.md` — new row: "knot stalls; rig silent for
  minutes" → the inactivity timeout stops the session and restarts it
  with a blocking-call note; if it recurs, check the hanging command or
  raise `inactivity-timeout-seconds`; use `pi-json` for blocked-call
  identification and session-resume restarts.
- `docs/release-notes.md` — v0.40.0 entry (feature): inactivity timeout
  with restart note; default adapter for **new** rigs is now
  `pi-json` (existing rigs keep their explicit setting); note that no
  rig-document migration is required.
- `knot-update` skill — **no migration entry** (additive config key).
- Bump `Cargo.toml` `0.39.0 → 0.40.0`; `cargo install --path .`.
- `project/plans/master-plan.md` — add row 81.

## Notes — design rationale

- **Why byte-level, not event-level, detection?** It works in both
  adapters with zero parsing coupling and matches the spec exactly
  ("no thinking, no response, no output"). JSON parsing happens only
  post-kill, on the already-accumulated buffer, for the session ID and
  the blocked-call name.
- **Why not just raise the total timeout?** The total timeout is a
  *budget*, not a stall detector — it cannot distinguish "still working"
  from "stuck". The two timers are orthogonal: inactivity bounds
  *silence*, total bounds *work*.
- **Why is a fresh restart (no session ID) safe?** Knots are designed
  to be idempotent; the restart re-sends the full prompt + strand
  reference. The note is what makes the second attempt different: the
  agent is told in advance how to keep the session alive (background +
  poll, or stream the output) instead of re-hanging silently.
- **Why `TimeoutSkipped` (not `Failed`) on exhaustion?** A deadline did
  fire and no final response was produced — identical to today's total
  timeout: the tie-off (agent output only) is preserved unchanged and
  the rig-log carries the operational event. `Failed` is for
  non-deadline terminal states (077's distinction).
- **Why no new rig-log event in v1?** `TimeoutExceeded` with the
  inactivity message keeps the rig-log schema stable, and external
  watchers already react to it. Revisit if a watcher needs to
  distinguish *stall* from *budget* programmatically — the loom-log
  `AgentInactivity` event already carries the structured detail.
- **Why the timer starts at spawn?** The pre-first-byte window is a
  real risk (cold provider, auth stall, pi startup hang); the spec
  counts "no thinking, no response, no output" from the moment the
  session started.
- **What "progress update" means for the agent.** With `pi-json`, a
  long bash command that *produces output* already survives (pi's
  100ms-throttled `tool_execution_update` stream). The note targets the
  remainder: commands with no output, or long waits between outputs —
  the agent's remedy is to run the task in the background and poll it
  (each poll is a tool call that produces output), or to stream the
  task's output.
- **Budget math.** Inactivity is per-attempt and unaffected by the
  elapsed budget; the total (remaining-budget) timeout still bounds each
  attempt and the whole loop. A profile with a 60s total and the 300s
  inactivity default simply never sees the inactivity timer (total
  fires first) — harmless.
- **Spurious-kill safety.** Stderr counts as activity; the 250ms poll
  granularity adds at most ~250ms to the window; and a child that exits
  on its own always wins over a simultaneous kill (status-first
  classification).

## Implementation Status: ✅ Complete (2026-09-03)

- **Verification (phase 6):** `cargo test --workspace` full suite
  green (948 lib + integration tests, 0 failures); `cargo clippy
  --all-targets` — the three warnings this plan's code introduced
  (`spawn_watchdog` arg count, two `.err().expect()` in tests) were
  fixed; the repo's pre-existing warnings are unchanged.
- **Empirical pi check (real binary, local model, `pi --mode json`):**
  - Direct stream — a `bash(sleep 120)` call emits
    `tool_execution_start` + one empty `tool_execution_update`, then
    silence (pi's own tool timeout is 150s, so the silence is the
    call, not pi) — the watchdog's input condition. A 12s outputting
    command streamed **121** `tool_execution_update` lines (the 100ms
    throttle reaches stdout in json mode) — the reset condition.
  - Full rig run (dev rig, inactivity 20s, `pi-json`, real model):
    `KnotProcessing → KnotEmptyResponse → SessionResumed(1) →
    AgentInactivity(2, silent=20s, window=20s, blocked=bash(sleep
    120)) → SessionResumed(2) → AgentInactivity(3, blocked=bash(<poll
    loop with no per-iteration echo>)) → SessionResumed(3) →
    KnotCompleted → StrandProcessed(error: null)` — tie-off produced
    with the session ID; rig-log held only `QueueIdle` (no
    exhaustion). The restart note changed behaviour on retry 3 (the
    model switched from a bare `sleep 120` to background + poll);
    the imperfect poll (silent loop) was killed once more, and the
    next re-entry completed. Both the kill-with-blocked-call and the
    success-after-restart stories are confirmed end-to-end.
- **Docs:** `docs/configuration/rig-structure.md` (config key, adapter
  table for the default flip, `AgentInactivity` loom event),
  `docs/concepts.md` (session-resume paragraph: the two timers, the
  restart note), `docs/troubleshooting.md` ("Knot Stalls — Rig Silent
  for Minutes"), `docs/release-notes.md` (v0.40.0 entry).
- **Skills:** `knot-init` 4.8.0 — step 5 documents the new default
  (`agent-adapter: pi-json`) and `inactivity-timeout-seconds`; fresh
  rigs get the full inactivity experience, existing rigs keep their
  explicit setting. `knot-update`: **no migration entry** (additive
  config key — old files parse with the 300 default).
- **Version:** `Cargo.toml` 0.39.0 → 0.40.0; installed with `cargo
  install --path .`.
- **Repo rule added (AGENTS.md):** agents must not test-run the Knot
  service in this repository — live rig runs are performed elsewhere
  (added after the empirical run above was started; the run had
  already completed and was captured as the evidence for this
  section).
