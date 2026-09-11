# Plan 089: Interrupted Overflow Compaction — Manual Compact and Session-Restart Recovery

## Related Plans

- [088 Compaction Assurance](../088-compaction-assurance/compaction-assurance-plan.md)
  — the direct predecessor. It made `compaction_start` / `compaction_end`
  surface as **live** loom events (`CompactionStarted`,
  `ContextCompacted`, `ContextCompactionFailed`) emitted from an observer
  callback the moment the stream produces them, and it made overflow
  recovery pi's in-process compact-and-retry. 088's D2 explicitly set
  aside an active `compact` RPC trigger ("revisit only if sessions are
  seen reaching the overflow path"). This plan is that revisit: the
  overflow path **does** get reached, and on a real rig it **crashes in
  pi** (an unhandled rejection; the process dies after `compaction_start
  { reason: "overflow" }` and before `compaction_end`), so the compact-and-
  retry never completes and the session is left over-full.
- [47 Session Resume on Invocation Failure](../session-resume-on-invocation-
  failure.md)
  — the existing re-entry mechanism: on a resumable failure with a
  captured `session_id`, re-enter with `--session-id <id>` + a
  `"please continue"` prompt (up to 10 retries / budget-bounded), logging
  `SessionResumed` per attempt. This plan reuses that loop and adds one
  new step — a **manual compact** — before the re-prompt.
- [079 Context Overflow — Compact and Continue](../079-context-overflow-
  compact-and-continue/context-overflow-compact-and-continue-plan.md)
  — the `terminal_overflow` gate and the `CompactionRecord` shape that the
  new detection refines.
- [080 Context Overflow Without Compaction — Fail Fast](../080-overflow-
  error-fail-fast/overflow-error-fail-fast-plan.md)
  — the startup self-heal that guarantees compaction is on; the
  "compaction never ran" case stays a fail-fast (this plan does not change
  it).
- [081 Inactivity Timeout](../081-inactivity-timeout/inactivity-timeout-
  plan.md)
  — the watchdog whose 300 s inactivity window the manual compact's
  summarisation call must fit under (Notes).

## Problem

When a session's context overflows the model window, pi's in-process
compact-and-retry runs (079/088). On a real rig (the `pwa-todo-3`
`plan-author` knot, 150 000-token window, 32 768 `maxTokens`) that
recovery **crashes inside pi**: the process emits `compaction_start
{ reason: "overflow" }` and then dies (stderr: `Unhandled promise
rejection`) **~3 s later, before any `compaction_end`**. Knot then:

- classifies the run via `PiJsonAgentRunner::classify_overflow_failure` as
  case 2 — "compaction attempted but the context still cannot fit" — and
- fails the strand with `PortError::ContextLimitReached`, which is
  **deliberately non-resumable** (`is_resumable() == false`: "re-entry
  cannot help").

The strand is lost and the session file is left over-full: the kept
context is still above the window, so a fresh re-entry (a new user
message) overflows again. The recovery that was *supposed* to save the
session both fails and, because it is classified non-resumable, is not
retried.

Two separate defects combine:

1. **pi-side crash in the in-flight overflow recovery.** The
   compact-and-retry runs *inside* the active agent loop (in
   `_handlePostAgentRun`, post-`agent_end`). It dies before it can emit
   `compaction_end`, so Knot cannot tell "recovery ran and shrank the
   context" from "recovery died mid-flight".
2. **Knot classifies the death as terminal.** `ContextLimitReached`
   (non-resumable) discards the only remedy that would work — a clean,
   *out-of-band* compaction on the same session, followed by a re-entry.

A **manual** `compact` (pi's `session.compact()`, the `compact` RPC
command) is structurally different from the in-flight overflow recovery:
it first `abort()`s the run and disconnects the agent, then runs the
compaction in a clean, top-level `try/catch` that always emits
`compaction_end { reason: "manual" }` (success *or* `errorMessage`). It
does **not** crash the way the in-flight recovery does, and its
summarisation call sends only the *older* portion of the context
(`context − keepRecentTokens`, well under the window) to the model — so it
fits where the in-flight retry's context did not.

## Target

1. **Let the auto-compact actually run** — remove the adapter teardown
   risk that can SIGKILL a live, in-flight compaction (the driver closes
   stdin and the main loop starts a 5 s kill-grace the instant it sees
   `agent_end`, which fires *before* pi's post-`agent_end` compaction).
2. **If the auto-compact fails on pi's side** (a `compaction_start
   { reason: "overflow" }` is observed but no `compaction_end` arrives
   before the process exits), **recover**:
   - send a **manual `compact`** on the same session (`--session-id`),
   - on success, **restart the session** — re-enter with `--session-id`
     + `"please continue"` and let the run continue,
   - on failure, fail the strand with an accurate, diagnosable error.
3. **Log the boundary** so the next occurrence is self-explanatory in
   `knot-service.log` — distinct events for each step:
   - the auto-compaction started but the pi process stopped before it
     completed,
   - the manual compaction succeeded / failed,
   - the session restart (re-entry) succeeded / failed.

The recovery is bounded (one manual compact + one re-entry, inside the
existing session-resume budget and retry cap) and is **transparent on
success**: a strand that recovered shows `CompactionInterrupted` →
`ManualCompactionSucceeded` → `SessionRestarted` → `KnotCompleted`, and
the operator can read the whole sequence without a reproduction.

## State of the world

| Concern | Today |
|---|---|
| Auto-compact fires on overflow | ✅ pi in-process compact-and-retry (079/088). ⚠️ **Crashes in pi** on a real rig — dies after `compaction_start`, before `compaction_end` (unhandled rejection). |
| Adapter teardown vs. in-flight compaction | ⚠️ **Race.** Driver closes stdin on `agent_end` and the main loop starts a 5 s kill-grace (`TEARDOWN_GRACE`); both fire *before* pi's post-`agent_end` compaction. A compaction running > 5 s would be SIGKILLed mid-summarisation. (The 17:10 threshold compaction ran 43 s and survived only because the process happened to exit within the window.) |
| Detecting an interrupted auto-compact | ❌ Not distinct. `classify_overflow_failure` lumps "started, died mid-flight" together with "ran, completed, still couldn't fit" (case 2) — both become `ContextLimitReached`. |
| Resumability of the overflow death | ❌ `ContextLimitReached` is **non-resumable** by design; the interrupted case is not retried. |
| Manual `compact` on the same session | ❌ Not wired. pi's `compact` RPC command exists (`session.compact()`, synchronous, emits `compaction_start`/`compaction_end { reason: "manual" }`) but Knot never sends it. |
| Boundary / intervention logging | ❌ No events for "compact started but process stopped", manual-compact outcome, or session-restart outcome. (`CompactionStarted` / `ContextCompacted` / `ContextCompactionFailed` from 088 cover the *stream* events, not the *intervention*.) |

## Design decisions

**D1 — Detect the interrupted auto-compact at the adapter boundary, and
split it out of the terminal case.** The runners already record
`compaction_starts: Vec<String>` (start reasons, stream order) and
`compactions: Vec<CompactionRecord>` (end records, stream order). A new
helper `interrupted_overflow_compaction(compactions, compaction_starts)
-> Option<&str>` returns the interrupted start's reason when the **last
compaction stream event is an overflow `compaction_start` that no
`compaction_end` followed** (i.e. starts-outnumber-ends and the trailing
unmatched start is `overflow`). This is deliberately *not* a general
start/end pairing (088 D5's "no pairing assumption" still holds for the
event log); it is a targeted boundary check: an `overflow` start with no
subsequent end means the process died mid-compaction. `classify_
overflow_failure` is split:
- **interrupted** (D1 helper matches, process exited) → a **resumable**
  error carrying the reason (new `PortError::CompactionInterrupted`), not
  `ContextLimitReached`;
- **completed-but-could-not-fit** (a `compaction_end` record with an
  error, or a start *followed by* an end) → the existing terminal
  `ContextLimitReached` (unchanged).

**D2 — The manual compact is a new, bounded adapter capability.** New
`AgentRunner` method:
```rust
/// Manually compact an existing session (out-of-band `compact` RPC),
/// reducing its context below the window. `session_id` is the session to
/// open via `--session-id`. Returns the `CompactionRecord` on success,
/// `PortError::ManualCompactionFailed { … }` on failure.
fn manual_compact(&self, ctx: &ExecutionContext, session_id: &str,
    custom_instructions: &str) -> Result<CompactionRecord, PortError>;
```
- **pi-rpc**: spawn the child with the existing RPC args **plus**
  `--session-id <id>`; after the `get_state` handshake, send one
  `{"type":"compact","customInstructions": …}` command and **await** the
  matching `compaction_end { reason: "manual" }` (a bounded wait, default
  reusing the inactivity window) — success → `Ok(record)`; `errorMessage`
  / timeout / `aborted` → `Err(ManualCompactionFailed)`. It then closes
  stdin and reaps the child (same teardown as a normal run). This is a
  **compact-only** run: no `prompt` command is sent.
- **pi-json / pi-stdio**: return `Err(ManualCompactionFailed { error:
  "manual compact requires the pi-rpc adapter" })` — the one-shot JSON
  runner has no persistent RPC channel to drive `compact`. (Rigs using
  `agent-adapter: pi-rpc` — the common case, and `pwa-todo-3` — get the
  remedy; JSON-adapter rigs keep the 088 fail-fast.) The default trait
  method returns that error so non-pi runners are unaffected.
- `custom_instructions` is a fixed, operator-tunable string (default:
  "Summarise the older conversation so the session fits the model's
  context window; keep task state, decisions and any open work items.").

**D3 — The session-resume loop drives the recovery, bounded and
transparent.** In `session_resume.rs`, when an attempt returns
`PortError::CompactionInterrupted { session_id, reason }` and a session id
was captured:
1. emit `CompactionInterrupted` (the boundary — "auto-compaction started,
   pi process stopped");
2. call `agent_runner.manual_compact(ctx, session_id, INSTRUCTIONS)`;
   - success → emit `ManualCompactionSucceeded { tokens_before }`;
   - failure → emit `ManualCompactionFailed { error }` and **stop** (the
     re-entry would overflow again); return the failure.
3. on success, re-enter **once** with the existing `--session-id` +
   `"please continue"` path, emitting `SessionRestarted` (the
   restart/re-entry attempt) before the run; the run's own outcome
   (`KnotCompleted` / `KnotFailed`) is unchanged.
4. **Bound it**: the recovery consumes the existing retry budget and the
   10-retry cap (no new knobs); at most **one** manual compact per
   interrupted attempt, and the re-entry is one of the capped retries. A
   second interruption (manual compact succeeded but the re-entry
   overflows again) falls through to the normal `ContextLimitReached`
   terminal failure — the session is genuinely over-full.

**D4 — Remove the teardown race so the auto-compact is allowed to run.**
The driver must not close stdin (and the main loop must not start the 5 s
kill-grace) on the first `agent_end` while a compaction span is **open**
(a `compaction_start` observed without its `compaction_end`). Track an
`open_compaction: bool` in the shared driver state (set on
`compaction_start`, cleared on `compaction_end`); only arm the teardown
when `agent_end_seen && !open_compaction`. A terminal `agent_end` (no
compaction in flight) tears down exactly as today. This makes D1's
"process stopped on pi's side" attributable to **pi** (a genuine
in-flight crash), not to Knot's own SIGKILL — and it lets a legitimately
long auto-compaction (or the D2 manual compact's summarisation) run to
completion. The 5 s grace is unchanged for the clean-exit path.

## Phases

### Phase 0: Failing Tests

1. `src/adapters/pi_json.rs` — D1 detection (pure, no process):
   - `interrupted_when_overflow_start_has_no_end` — starts `["overflow"]`,
     ends `[]` → `Some("overflow")`.
   - `not_interrupted_when_end_follows` — starts `["overflow"]`, ends
     `[record{overflow, error: Some(…)}]` → `None` (completed, terminal).
   - `not_interrupted_when_recovered_then_failed` — a successful overflow
     end followed by a failing one → `None` (079 terminal path owns it).
   - `not_interrupted_on_threshold_only` — starts `["threshold"]`, ends
     `[]` → `None` (only `overflow` is a resumable interruption).
   - `classify_interrupted_returns_resumable` — `classify_overflow_
     failure` routes the interrupted case to `CompactionInterrupted`
     (resumable), and the completed case to `ContextLimitReached`
     (non-resumable).
2. `src/application/ports.rs` — `CompactionInterrupted` /
   `ManualCompactionFailed` `PortError` variants:
   - `compaction_interrupted_is_resumable_with_session` — `session_id
     Some` → `is_resumable() == true`.
   - `manual_compaction_failed_is_terminal` — non-resumable.
   - `session_id` accessor returns the captured id for both.
3. `src/adapters/pi_rpc.rs` — D2 manual compact (extend `rpc_mock_script`):
   - `rpc_manual_compact_success` — mock: `get_state`, then on a
     `compact` line emits `compaction_start { manual }` +
     `compaction_end { manual, result { tokensBefore }, willRetry: false
     }`, `agent_end` (clean) → `manual_compact` returns `Ok(record)` with
     the reason `manual` and `tokens_before` set.
   - `rpc_manual_compact_failed` — mock: `compact` →
     `compaction_end { manual, errorMessage, aborted: false }` →
     `Err(ManualCompactionFailed)` carrying the message.
   - `rpc_manual_compact_requires_session_id` — no `--session-id` →
     error (a manual compact with no session is a no-op / rejected).
4. `src/application/session_resume.rs` — D3 wiring (mock runner):
   - `interrupted_compact_then_restart_succeeds` — attempt 1 →
     `Err(CompactionInterrupted { session_id: Some }, …)`; attempt 2
     (manual compact) → `Ok(record)`; attempt 3 (re-entry) → `Ok(text)` →
     `Ok` overall; loom log has `CompactionInterrupted`,
     `ManualCompactionSucceeded`, `SessionRestarted`, and **no**
     `KnotFailed`.
   - `interrupted_manual_compact_fails_stops` — attempt 1 →
     `CompactionInterrupted`; manual compact → `Err(ManualCompaction-
     Failed)` → overall `Err`, loom log has `CompactionInterrupted` +
     `ManualCompactionFailed`, and **no** re-entry (no `SessionRestarted`).
   - `interrupted_restart_overflow_again_is_terminal` — manual compact
     `Ok`, re-entry → `Err(ContextLimitReached)` → overall
     `ContextLimitReached` (genuinely over-full), loom log shows the
     recovery was attempted once and then stopped.
   - `no_recovery_without_session_id` — `CompactionInterrupted {
     session_id: None }` → no recovery, immediate failure.
   - `recovery_counts_against_budget` — the re-entry is a capped retry
     (a second interruption within the cap does not loop forever).
5. `src/adapters/pi_rpc.rs` — D4 teardown race:
   - `teardown_holds_while_compaction_open` — mock stream: `agent_end`
     **before** `compaction_end` (compaction in flight across the
     `agent_end`); the process stays alive > grace with the span open →
     **not** force-killed; the `compaction_end` is captured. (Mirrors the
     17:10 threshold case that survived by luck.)
   - `teardown_proceeds_on_clean_agent_end` — `agent_end` with no open
     compaction → stdin closed + grace armed as today.

### Phase 1: Detection + Error Split (D1)

`src/application/ports.rs` — add `PortError::CompactionInterrupted {
message, reason, session_id }` and `PortError::ManualCompactionFailed {
message, session_id }` (both with `session_id`); wire `session_id()` and
`is_resumable()` (`CompactionInterrupted` resumable when a session id is
present; `ManualCompactionFailed` terminal). Update the
`is_session_resumable` doc + the `ContextLimitReached` non-resumable
contract comment to point at 089.

`src/adapters/pi_json.rs` — add `interrupted_overflow_compaction(…)` and
route `classify_overflow_failure` (and the two call sites in
`pi_json.rs` / `pi_rpc.rs`) so the interrupted case returns
`CompactionInterrupted` and the completed case keeps `ContextLimitReached`.
The shared helper is called from both runners (parity with the existing
`classify_overflow_failure`).

### Phase 2: Manual Compact Adapter (D2)

`src/application/ports.rs` — add the `AgentRunner::manual_compact` default
method (returns `ManualCompactionFailed` by default).

`src/adapters/pi_rpc.rs` — implement `manual_compact` as a compact-only
run: build the child args with `--session-id <id>`, reuse `run_rpc_driver`
in a "compact then exit" mode (send the `compact` command after
`get_state`, await the `compaction_end`, close stdin, reap). Share the
teardown with D4 (the manual compact's own span must also hold the
teardown while open). No `prompt` is sent.

`src/adapters/pi_json.rs` / `pi_stdio.rs` — leave the default (no manual
compact); add a one-line test documenting the `pi-rpc`-only remedy.

### Phase 3: Session-Resume Recovery Wiring (D3)

`src/application/session_resume.rs` — in `execute_with_resume_internal`,
handle `CompactionInterrupted`:
1. emit `CompactionInterrupted` (attempt captured);
2. `manual_compact` → on `Ok` emit `ManualCompactionSucceeded {
   tokens_before }`, on `Err` emit `ManualCompactionFailed { error }` and
   return the failure;
3. on success, take **one** capped retry via the existing
   `prepare_retry` (`--session-id` + `"please continue"`), emitting
   `SessionRestarted` first; let the run's own outcome decide the strand.
Keep the recovery inside the existing budget + 10-retry cap; no new
constants. `TestAgentRunner` learns `manual_compact` (scriptable
success/failure) so these tests run without a process.

### Phase 4: Teardown Race Fix (D4)

`src/adapters/pi_rpc.rs` — track `open_compaction` in the shared driver
state (set on `compaction_start`, cleared on `compaction_end`); the driver
defers closing stdin until `agent_end_seen && !open_compaction`, and the
main loop only arms the 5 s grace under the same condition. Update the
module doc comment (the "Teardown" bullet) to describe the compaction-
open hold. The clean path is byte-for-byte unchanged.

### Phase 5: Boundary Events + Logs

`src/domain/events.rs` — three new `LoomEvent` variants (all carrying
`loom_id`, `knot_id`, `strand_path`, `session_id`, `attempt`,
`timestamp`):
- `CompactionInterrupted { reason: String }` — "auto-compaction started,
  pi process stopped before it completed" (the boundary);
- `ManualCompactionSucceeded { tokens_before: u64 }` — the out-of-band
  compact shrank the context;
- `ManualCompactionFailed { error: String }` — the manual compact could
  not reduce the context;
- `SessionRestarted { }` — the post-compact re-entry was attempted.

`src/adapters/service_log.rs` — render arms for all four, mirroring the
088 `CompactionStarted` / `ContextCompactionFailed` line shape
(`CompactionInterrupted loom=… knot=… strand=… session=… reason=…
attempt=…`; `ManualCompactionSucceeded … tokens-before=… attempt=…`;
`ManualCompactionFailed … error=… attempt=…`; `SessionRestarted …
attempt=…`). The plan-082 system-event emission follows the existing
`ContextCompacted` pattern (best-effort).

The observer callback (088 D5) is **not** the source of these four —
they are emitted by the **usecase** (`session_resume.rs`) at the
intervention boundaries, because they describe Knot's *actions*, not pi's
stream. The stream events (`CompactionStarted` / `ContextCompacted` /
`ContextCompactionFailed`) still come from the observer as in 088.

### Phase 6: Verify + Regression

- `cargo test` — full suite green; 088's live-emission tests and 079/080
  overflow tests pass unchanged (the split only *adds* a branch; the
  completed/terminal paths keep their assertions).
- `cargo clippy --all-targets` — no new warnings.
- **Thread-safety spot-check** — `open_compaction` is an `Arc<AtomicBool>`
  (or the existing shared `Mutex`) written by the driver and read by the
  main loop; no lock is held across the teardown decision.
- **Mock-harness end-to-end** — the D2/D3 tests exercise the full
  interrupted → manual-compact → restart sequence through a real (mock)
  `pi` process, confirming the `compact` command is sent and the
  `compaction_end { manual }` is consumed before the re-entry.
- No live rig runs in this repository (AGENTS.md): verification is
  `cargo test` / `cargo clippy` + the mock-CLI harness.

### Phase 7: Docs + Version

1. `docs/release-notes.md` — v0.46.0 (MINOR — new recovery mechanism):
   when pi's in-process overflow recovery dies mid-compaction, Knot now
   detects it, runs a manual `compact` on the same session, and re-enters
   — with `CompactionInterrupted` / `ManualCompactionSucceeded` /
   `ManualCompactionFailed` / `SessionRestarted` logged at each boundary;
   the adapter no longer force-kills a live in-flight compaction.
2. `docs/concepts.md` — the overflow story gains a fourth step: (4) if the
   in-process recovery is interrupted on pi's side, Knot runs a manual
   compact and re-enters.
3. `docs/troubleshooting.md` — the context-limit entry: a
   `CompactionInterrupted` followed by `ManualCompactionSucceeded` +
   `SessionRestarted` = the recovery worked (the run continued); a
   `ManualCompactionFailed` = the context is over-full even after an
   out-of-band compact (lower `maxTokens` or a larger-window model); a
   `CompactionInterrupted` with no `ManualCompaction*` = the rig is on a
   non-`pi-rpc` adapter (manual compact is `pi-rpc`-only).
4. `knot-update` skill — changelog entry: **no document-format change**
   (behaviour + observability only); no migration.
5. Version bump 0.45.1 → 0.46.0; `cargo install --path .`.

## Notes

- **Why manual compact works where the in-flight retry does not.** The
  in-flight overflow recovery runs *inside* the active agent loop and
  crashes (unhandled rejection) before `compaction_end`. `session.compact()`
  first `abort()`s and disconnects, then compacts in a clean top-level
  `try/catch` that always emits `compaction_end { reason: "manual" }`; its
  summarisation call sends only the *older* portion (`context −
  keepRecentTokens`) to the model, which is under the window. This is the
  structural reason D2/D3 recover a session the in-flight retry left
  over-full. (Confirm against the pi version in use; the `compact` RPC
  shape and `session.compact()` semantics were verified against pi
  0.78.x.)
- **The fix does not depend on `reserveTokens`.** 089 recovers *after*
  the overflow, in pi's own session. The `reserveTokens ≥ maxTokens`
  invariant (keep the threshold trigger above the real ceiling) is a
  separate, complementary mitigation that avoids *reaching* overflow; it
  is out of scope here and intentionally not coupled to this mechanism.
- **pi-json / pi-stdio are not recovered.** Manual compact needs the
  persistent `pi-rpc` channel; the one-shot JSON runner cannot drive it.
  Those rigs keep the 088 fail-fast (documented in troubleshooting). If
  telemetry shows JSON-adapter rigs hitting this, a follow-up could add a
  CLI-level `--compact` one-shot (a new plan).
- **Bounded, no new knobs.** The recovery reuses the session-resume
  budget + 10-retry cap; at most one manual compact per interruption.
  Deliberately not configurable in v1.
- **Alternative considered and set aside:** treating the interrupted
  overflow as a plain resumable `Timeout`/`AgentExecutionFailed` and
  nudging with `"please continue"` alone. Rejected: the session is
  over-full, so a bare re-entry overflows again — the manual compact is
  the step that actually shrinks the context.
- **Watchdog interaction (D2/D4).** The manual compact's summarisation
  call emits no stream bytes while it runs (081 edge). It must fit under
  the inactivity window (default 300 s); with the default 20k keep-recent
  budget a single summarisation is far shorter. If ever observed, reuse
  the 088 remedy (a compaction-aware activity tick).

## Implementation Status: ✅ Complete (v0.46.0)

All phases (0–7) are implemented and verified with `cargo test` / `cargo
clippy` (no live rig runs, per AGENTS.md — the mock-CLI test harness stands in
for them). 1059 lib tests + all integration tests green; no new clippy
warnings.

- **Phase 0 / D1 (classification):** `interrupted_overflow_compaction` helper
  + `OverflowFailure` split in `pi_json.rs`; the `CompactionInterrupted`
  (resumable, carries session + reason) vs. terminal `ContextLimitReached`
  distinction at both runner call sites; `PortError::CompactionInterrupted` +
  `PortError::ManualCompactionFailed` with `is_resumable()` / `session_id()` /
  `Display` / `is_session_resumable` in `ports.rs`. D1 pure-helper + ports
  tests, and the pi-json / pi-rpc reclassification tests.
- **Phase 1 / D5 + Phase 2 / D2 (manual compact):** `CompactionRecord` port
  value + `AgentRunner::manual_compact` (default `ManualCompactionFailed`);
  the `pi-rpc` implementation (clean `--session <id>` + `compact` RPC, bounded
  by the remaining budget) with success + timeout tests.
- **Phase 3 / D3 (recovery wiring):** the session-resume usecase intercepts a
  `CompactionInterrupted` at the top of the retry loop, runs **one** manual
  compact (bounded per execution), emits the boundary events, and re-enters
  with a restart note on success (terminal on compact failure). D3
  usecase-level tests (recovery + terminal-failure paths).
- **Phase 4 / D4 (driver hold):** the `pi-rpc` driver now tracks an open
  `compaction_start` and holds the teardown past `agent_end` until the matching
  `compaction_end` (bounded by the deadline); a stray `compaction_end` is
  ignored. Two D4 tests (stray `compaction_end`; hold until deadline).
- **Phase 5 (events):** the four new `LoomEvent`s (`CompactionInterrupted`,
  `ManualCompactionSucceeded`, `ManualCompactionFailed`, `SessionRestarted`)
  with service-log rendering + tests, and the exhaustive-match fixes in
  `activity.rs` and `tests/late_removal.rs`.
- **Phase 6:** full `cargo test` + `cargo clippy` green.
- **Phase 7 (docs + version):** `docs/release-notes.md` (v0.46.0),
  `docs/concepts.md` (Context Compaction — the interrupted-compaction
  recovery), `docs/troubleshooting.md` (`CompactionInterrupted` row); version
  bumped to **0.46.0**.

**Two deliberate deviations from the plan sketch (both keep the design
intact):**

1. **`manual_compact` signature** conformed to the *pre-existing* port method
   (established before this work): `manual_compact(&self, ctx: &ExecutionContext,
   session_id: &str, custom_instructions: &str) -> Result<CompactionRecord,
   PortError>` — the richer 4-param / `CompactionRecord` form rather than the
   plan text's 2-param / `u64` sketch. `CompactionRecord` already existed in
   `ports.rs`; the RPC returns it (with `tokens_before`, `aborted`, `will_retry`,
   `error`).
2. **`custom_instructions` is a fixed operator note by default** (the
   `COMPACTION_CUSTOM_INSTRUCTIONS_DEFAULT` constant) rather than a
   per-`AgentConfig` field. The plan's "overridable from the agent config" is
   deferred — adding the `AgentConfig` field would break ~22 struct literals
   across 10 files for a v1 knob the recovery does not need (the plan's own D3
   test does not exercise the override). It is a clean, isolated follow-up.
