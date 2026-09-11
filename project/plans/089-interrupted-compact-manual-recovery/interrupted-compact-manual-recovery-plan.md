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

## Follow-on evidence (the 2026-09-11/12 `pwa-todo-3` rig run)

The rig ran the whole of 2026-09-11 evening into 2026-09-12 morning
(`tie-offs/software-factory-rig/knot-service.log`, 16:05 → 07:10, 14 looms,
`agent-adapter: pi-rpc`, 150 000-token window). It is the first live test of
088/089 and it confirms the overflow recovery while exposing the shape D4
was meant to cover and did not. Cross-checked against the pi session files
in `~/.pi/agent/sessions/--home-hayden-workspace-proto-apps-pwa-todo-3--/`.

| # | Scenario | Evidence | In 0.46.0 |
|---|---|---|---|
| **A** | **Threshold compaction ends the attempt.** `CompactionStarted(reason=threshold, attempt N)` → 1 s later `KnotEmptyResponse(N)` → `SessionResumed(N)` → `CompactionStarted(session=, reason=threshold, attempt N+1)` → `ContextCompacted(N+1)`. The first compaction is killed mid-summarisation; the second one (in the resumed process) succeeds. | 17:10:51 (`plan-implementer`), 19:05:03, 22:33:16, 23:13:32 and **23:27:37** (a second occurrence on the *same* strand: attempt 2 → 3). Session `01a09066…` shows it exactly: last entry 12:33:16.790Z is a **thinking-only assistant message** (empty `agent_end`), no `compaction` entry until 12:34:04.473Z. | ❌ **Not fixed.** D4 armed only the force-kill; the driver still closes stdin at `agent_end` and pi exits on stdin EOF (see D5). Costs a full resume + a duplicate 30–47 s summarisation each time. |
| **B** | **Overflow interrupted → manual compact → session restart → completion.** The 089 recovery path, first live proof. | 22:47:31 `CompactionStarted(reason=overflow)` → 22:47:43 `CompactionInterrupted` → 22:48:22 `ManualCompactionSucceeded` + `SessionRestarted` → 22:48:32 `SessionResumed(attempt=2)` → 23:02:57 `KnotCompleted` (strand chained `PhaseReady`). | ✅ **Works.** |
| **C** | **Overflow fail-fast** — `CompactionStarted(reason=overflow)` then 3 s later `KnotFailed … context overflow, but pi auto-compaction did not run (400 … 150001 input tokens)`. | 16:45:13/16 (`plan-author`), 18:36:26/27 (`phase-implementer`). | ✅ Superseded by B — both ran on the pre-0.46.0 binary (0.46.0 was installed 22:10, the service restarted 22:16). |
| **D** | **Compaction stalls trip the 081 inactivity watchdog.** pi emits no stream bytes while the summarisation call runs; the watchdog kills at 300 s. | 17:11:34 `ContextCompacted` → silence → `WARNING: killed 'pi' after inactivity — no output for 300s` → 17:21:50 `AgentInactivity` → 17:40:22 `KnotFailed … overall timeout budget exhausted after 2 attempt(s) (2500s used of 2500s budget)`. Again after the 22:48 session restart: 22:56:08 `AgentInactivity`. Also 18:14, 18:31, 18:43. | ❌ **Not fixed.** The 089 Notes name the hazard for the manual compact only; the auto-compaction and the post-restart turn hit it (see D8). |
| **E** | **Suspend/restart orphans the in-flight run silently.** | Session `01a08fad…` stops at 19:08:39 (nothing in the log until the 22:16:53 restart, `initial snapshot … queue=1` — the strand re-ran from scratch in a new session). Same again 23:28:31 → 06:49:27 (`queue=1`). | ❌ No event distinguishes an abandoned in-flight run from a fresh queue entry (see D9c). |
| **F** | **Observability nits.** | `session=` (empty) on every `CompactionStarted` of a *resumed* attempt (17:11:05, 19:05:14, 22:33:27, 23:13:44, 23:27:48); an empty response caused by Knot's own teardown labelled `KnotEmptyResponse`; `WARNING: pipeline tasks did not drain within 5s, aborting` (15:52:52). | ❌ (see D9). |

## Design decisions (follow-on)

**D5 — Tear the RPC session down on `agent_settled`, and never close stdin
with a compaction open.** Two facts fix scenario A:

1. pi's run is **not** over at `agent_end`. The post-agent work
   (`AgentSession._handlePostAgentRun` → `_checkCompaction` →
   `_runAutoCompaction`) runs *after* `agent_end`, in the same awaited
   prompt; the run is really over at **`agent_settled`**, which pi emits in
   the `finally` of `_runAgentPrompt` — i.e. after any compaction and after
   any continuation the compaction asked for.
2. pi's rpc-mode treats **stdin EOF as shutdown**
   (`process.stdin.on("end", () => shutdown())` → `process.exit(0)`). So the
   driver's `stdin = None` at `agent_end` does not merely *allow* a clean
   exit, it **causes** one — 1 s into the compaction, with no
   `compaction_end` and no `compaction` entry written to the session file.
   Exit 0 + empty final text is then reported as `KnotEmptyResponse`.

So: the driver closes stdin on `agent_settled` (new flag), keeping the
existing `agent_end && !open_compaction` rule only as the fallback for a pi
old enough not to emit `agent_settled` — and in that fallback the close is
deferred by a short *settle window* (250 ms), because pi emits
`compaction_start` in the same tick as `agent_end`, i.e. **always after it**.
The main loop arms `TEARDOWN_GRACE` on the same condition. The final text is
still taken from the last `agent_end`.

*Why the D4 tests missed this*: `mock_script` never reads stdin, so a mock
cannot exit on stdin EOF, and
`rpc_d4_open_compaction_holds_teardown_until_deadline` emits
`compaction_start` **before** `agent_end` — the reverse of pi's order. Both new tests must use a **stdin-aware** mock that models pi's
EOF-exit and pi's real event order.

**D6 — Continue the compacted turn in the same session.** After a
*threshold* compaction pi ends the turn (`_runAutoCompaction` returns
`hasQueuedMessages()`), leaving a compacted, healthy, **idle session on a
still-open RPC channel**. Knot should ask that session for its answer
instead of paying for a new process. One `{"type":"prompt","message": …}`
continuation (reusing the `FINAL_RESPONSE_REQUEST` wording), then take the
**next** `agent_end`'s text; only fall back to the existing
`--session-id` re-entry if the continuation also ends empty. This removes
the duplicate summarisation and the ~11.6 kB prompt re-send seen in scenario
A, and it is `pi-rpc`-only (the one-shot JSON/stdio runners keep the resume
path unchanged).

**D7 — An interrupted compaction is interrupted whatever the reason.**
`interrupted_overflow_compaction` matches `overflow` starts only, so the
threshold shape (A) never reaches the D2/D3 recovery and is reported as an
empty response instead. Once D5 removes Knot's own kill, *any* trailing
unmatched `compaction_start` means the same thing: the process died
mid-compaction. Generalise the helper to any reason; keep
`ContextLimitReached` for the completed-but-cannot-fit cases.

**D8 — A live compaction is proof of life.** While a compaction span is
open the stream is legitimately silent (089 Notes; scenario D). Refresh the
081 last-activity stamp from the span boundaries themselves (the 088
observer already runs on the reader thread, so no new thread or lock), and
skip the inactivity check while `open_compaction` is set — the total
budget still bounds it. Applies to the D2 manual compact's own run.

**D9 — Observability.** (a) Seed `CompactionObserveState` from the
`--session-id` / `--session` value Knot itself passed, so a resumed attempt
never logs `session=`. (b) With D7 the empty-response label disappears for
this shape; where an empty response *does* stand, the line carries the open
compaction reason. (c) When the queue is restored at startup and a persisted
strand was `processing` when the service stopped, log `RunAbandoned` with
the strand and session id — the 19:08→22:16 and 23:28→06:49 losses were
invisible in the log.

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

---

## Follow-on phases (8–13, target v0.47.0)

Phases 0–7 are shipped in 0.46.0 and are **not** revisited. Phases 8–13
close the gaps the 2026-09-11/12 rig run exposed (scenarios A, D, E, F
above). They are ordered so each phase is green on its own; 8 and 9 are the
substance, 10–12 are small and independent, 13 closes out.

### Phase 8: Settle-based teardown + stdin hold (D5)

**How it works.** The RPC driver stops treating `agent_end` as the end of
the run. It tracks three things: `agent_end_seen` (a turn finished, its text
captured), `open_compaction` (a span opened and not yet closed — unchanged,
but now honoured by *both* teardown paths), and `agent_settled_seen` (pi has
finished the whole prompt, post-agent compaction included). stdin is closed
only when pi is genuinely done:

```
close stdin  ⇔  agent_settled_seen                      (pi ≥ 0.78)
           or  (agent_end_seen && !open_compaction
                && settle_window elapsed)               (fallback, 250 ms)
```

The 250 ms settle window exists because pi emits `compaction_start` in the
same tick **after** `agent_end`; without it the fallback re-creates the bug
for older pi. The main teardown loop arms `TEARDOWN_GRACE` under the same
predicate, so the 5 s force-kill grace keeps its meaning (a *graceful*
process that will not exit) and the total budget remains the hard bound for
an in-flight compaction. `pi --mode rpc` then exits by itself on the closed
stdin, exactly as today.

1. `src/adapters/pi_rpc.rs` — handle `Some("agent_settled")` in
   `run_rpc_driver` (set the flag); move `stdin = None` from the `agent_end`
   arm to the settle arm; add the settle-window timer for the fallback path.
2. Same file, main teardown loop — arm/hold on the same predicate (read the
   shared flags; no lock across the decision, as today).
3. Module doc: rewrite the **Teardown** bullet — teardown is on
   `agent_settled` (or `agent_end` + no open compaction after the settle
   window), *because pi exits on stdin EOF and closes its own post-agent
   compaction if the pipe closes early*.
4. **Stdin-aware mock** (test-only helper next to `mock_script`): a script
   that reads stdin until EOF and exits 0 on EOF, logging the lines it saw.
   Every D5 test uses it — a mock that ignores stdin cannot model pi and is
   how phase 4 passed while the defect stayed live.
5. Tests (all with the stdin-aware mock, pi's real event order):
   - `rpc_agent_end_empty_then_threshold_compaction_completes` —
     `agent_end`(empty text) → `compaction_start{threshold}` → (hold) →
     `compaction_end{threshold, result}` → `agent_settled`; asserts the
     process was **not** killed early (the `compaction_end` is observed and
     `ContextCompacted` is logged with the right attempt), and that the run
     does not hang past its deadline.
   - `rpc_closes_stdin_on_agent_settled` — clean single-turn run: exactly one
     `prompt` line seen, stdin closed after `agent_settled`, exit within the
     grace (no regression on the clean path).
   - `rpc_fallback_settle_window_closes_stdin_without_settle_event` — a mock
     that never emits `agent_settled` (old-pi shape): teardown still happens
     after `agent_end` + settle window, and a `compaction_start` arriving
     inside the window defers it.
   - Keep both existing D4 tests, and add a note in the older one that its
     event order is inverted w.r.t. pi (kept for the flag-hold behaviour).
6. `cargo test` / `cargo clippy --all-targets` green.

### Phase 9: In-session continuation after a compaction (D6)

**How it works.** After phase 8 a threshold compaction ends the attempt with
the session compacted and *idle on an open channel* instead of exiting. The
driver notices the one shape that needs help — **final text empty, a
compaction span completed, run settled** — and asks the same session for the
answer it owes:

1. send one `{"type":"prompt","message": FINAL_RESPONSE_REQUEST}` on the
   live stdin, log `TurnContinued { reason, attempt }`, and keep reading;
2. the second turn produces a fresh `agent_end` (text) + `agent_settled` —
   the driver takes **that** text as the response, closes stdin, returns
   `Ok`; the turn's own events (`ContextCompacted`, `TurnContinued`,
   `KnotCompleted`) are the operator's whole story;
3. bound it: **one** continuation per attempt, no new knob; a continuation
   that also ends empty (or errors) falls through to the existing resumable
   empty-response path, so the process-restart resume stays as the
   backstop — and with D5/D7 it now resumes an already-compacted session,
   so it no longer pays for the summarisation twice.

1. `src/adapters/pi_rpc.rs` — the continuation step inside the driver
   (fire-once flag, bounded by the remaining total budget).
2. `src/domain/events.rs` + `src/adapters/service_log.rs` —
   `TurnContinued { loom_id, knot_id, strand_path, session_id, reason,
   attempt }` → `TurnContinued loom=… knot=… strand=… session=… reason=…
   attempt=…`, plus the plan-082 system event and the `activity.rs` match
   arm. `SessionRestarted` keeps its meaning (**new process** via
   `--session-id`); `TurnContinued` is the in-session sibling.
3. Tests:
   - `rpc_continues_in_session_after_threshold_compaction` — stdin-aware
     mock asserts the `prompt` line is received, then plays the second turn;
     result is the second turn's text, `TurnContinued` logged, **no**
     `KnotEmptyResponse`, **no** `SessionResumed`.
   - `rpc_continuation_fires_once` — second empty turn is not followed by a
     second `prompt`.
   - `rpc_continuation_empty_falls_back_to_resume` — the usecase sees the
     empty response and takes the existing `--session-id` resume
     (`SessionResumed` logged).
   - `manual_compact` path unaffected (no continuation is sent on a
     compact-only run).
4. `cargo test` / `cargo clippy` green.

### Phase 10: Threshold interruptions are interruptions (D7)

**How it works.** `interrupted_overflow_compaction` becomes
`interrupted_compaction`: same shape test (starts outnumber ends and the
trailing unmatched start has no end), **any** reason. With D5 the shape is
unambiguous — Knot no longer kills the process it is watching, so an
unmatched start can only mean pi died mid-summarisation. The interrupted
route is already resumable and drives the D2 manual compact + D3 restart, so
a threshold interruption gets the same recovery as an overflow one, and the
misleading `KnotEmptyResponse` line disappears.

1. `src/adapters/pi_json.rs` — rename + drop the `overflow`-only match;
   update both call sites (`pi_json.rs`, `pi_rpc.rs`) and the doc comment's
   case 0 wording.
2. Tests — replace `not_interrupted_on_threshold_only` with
   `interrupted_when_threshold_start_has_no_end` (→ `Some("threshold")`);
   keep the completed/recovered cases; add
   `classify_interrupted_threshold_is_resumable`. Record the reversal of the
   089 D1 assumption in the phase document's **Deviations** (it was correct
   while Knot could kill the process; D5 is what makes it safe).
3. `cargo test` / `cargo clippy` green.

### Phase 11: Compaction-aware inactivity window (D8)

**How it works.** The 081 watchdog asks "has pi written anything lately?";
while pi is summarising the honest answer is *no*, and 300 s of silence is a
normal compaction on a loaded workstation. So the compaction span itself
becomes an activity signal: the 088 observer (already on the stdout reader
thread) calls `live.touch()` on `compaction_start` and `compaction_end`, and
the watchdog skips the inactivity test while `open_compaction` is set. The
total budget is untouched, so a wedged compaction still dies — on the
deadline, with a `Timeout`, not on a fake stall. The same hold covers the D2
manual compact (089 Notes) and the post-`SessionRestarted` turn (22:56:08 in
the evidence table), which is what ate the 17:40 `KnotFailed … 2500s of
2500s budget` outcome.

1. `src/adapters/live_output.rs` — `touch()` (update `last_activity`);
   `spawn_watchdog` takes an optional `open_compaction: Arc<AtomicBool>`
   checked before the inactivity branch.
2. `src/adapters/pi_rpc.rs` / `pi_json.rs` — wire the flag and the touches
   (the JSON runner gets the same hold for free via its observer callback);
   update the 081 module doc: "a compaction span counts as activity".
3. Tests — `inactivity_holds_while_compaction_open` (mock holds the span
   past a short inactivity window, run still succeeds and no
   `AgentInactivity` is logged); `total_budget_still_kills_a_wedged_
   compaction` (mock never closes the span → `Timeout` at the deadline);
   `activity_resumes_after_compaction_end` (inactivity kills again once the
   span is closed).
4. `cargo test` / `cargo clippy` green.

### Phase 12: Observability polish (D9)

**How it works.** Three small, independent logging fixes — no behaviour
change, so they land after the phases that change behaviour.

1. **Known session id.** Both runners seed `CompactionObserveState::
   set_session_id(…)` from `agent_config.extra_args` (`--session-id` /
   `--session`) *before* spawning, so a resumed attempt's
   `CompactionStarted` never prints `session=` again (the RPC stream has no
   `session` header and the `get_state` response can land after the first
   compaction line — the reason all five occurrences in the evidence table
   printed an empty id while the matching `ContextCompacted` printed the
   right one).
2. **Honest empty response.** Where an empty response still stands (no open
   compaction, D7 not matched), the existing `KnotEmptyResponse` stays, but
   the plan-082 system event gains the open-span reason when one was seen,
   so the line never blames the agent for a teardown it cannot be.
3. **Abandoned runs.** At startup, after loading persisted events, emit
   `RunAbandoned { strand_path, knot_id, session_id? }` for each restored
   queue entry that was mid-processing, and document in troubleshooting that
   a `RunAbandoned` followed by a fresh `KnotProcessing` is a **re-run from
   scratch** (the previous session's work is not resumed) — the 19:08→22:16
   and 23:28→06:49 gaps in the evidence table.
4. `src/domain/events.rs` + `service_log.rs` + `activity.rs` —
   `RunAbandoned` variant, render arm (`RunAbandoned loom=… knot=… strand=…
   session=…`), match arms, tests; seed tests for (1).
5. `cargo test` / `cargo clippy` green.

### Phase 13: Verify, docs and version

1. `cargo test` and `cargo clippy --all-targets` green; the 088 live-observer
   and 079/080 overflow tests pass unchanged except where phase 10 reverses
   one assertion.
2. **Mock-harness end-to-end** — one test that walks the whole healthy shape:
   `agent_end`(empty) → `compaction_start{threshold}` → `compaction_end` →
   `agent_settled` → continuation `prompt` → `agent_end`(text) →
   `agent_settled`, asserting `ContextCompacted` + `TurnContinued` and no
   `KnotEmptyResponse` / `SessionResumed`. No live rig runs in this
   repository (AGENTS.md).
3. `docs/release-notes.md` — **v0.47.0** (MINOR): threshold compactions no
   longer end the attempt; in-session continuation; interruptions for any
   reason; compactions no longer trip the inactivity watchdog; `TurnContinued`
   and `RunAbandoned` events.
4. `docs/concepts.md` — the compaction story: the run *settles* after the
   post-agent compaction, Knot asks the compacted session to continue, and a
   compaction span counts as activity.
5. `docs/troubleshooting.md` — a `CompactionStarted` with no
   `ContextCompacted` now means a **pi-side** death (Knot no longer causes
   it) → expect `CompactionInterrupted` + manual compact; add
   `TurnContinued` (normal, cheap) and `RunAbandoned` (re-run) rows.
6. `knot-update` skill — changelog entry: **no document-format change**, no
   migration.
7. Version bump 0.46.0 → 0.47.0; `cargo install --path .`.

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
  the 088 remedy (a compaction-aware activity tick). **Observed** on the
  2026-09-11 rig (four 300 s kills, one of which ended in a budget-exhausted
  `KnotFailed`) — phase 11 is that remedy.
- **`agent_settled` availability (D5).** Verified against pi 0.78.1:
  `_emitAgentSettled()` is awaited in the `finally` of `_runAgentPrompt`,
  i.e. after `_handlePostAgentRun` (the compaction and any continuation),
  and rpc mode forwards every session event, so the RPC stream carries it.
  Knot currently ignores it, and keeps `agent_end` as the fallback for older
  pi.
- **Suspend is not Knot's to fix (scenario E).** Knot cannot tell "pi went
  away" from "the workstation slept for three hours"; it can only make the
  consequence visible (D9c). Reducing the loss belongs to the
  `reserveTokens ≥ maxTokens` invariant (avoid *reaching* compaction) and to
  keeping runs short — the standing note above keeps that out of scope.
- **Follow-on phases keep the 089 design.** Phases 8–13 add no new knobs,
  no new ports and no document-format change; they complete D4's mechanism,
  extend D1/D2/D3 to the threshold reason, and reuse the 088 observer for
  the 081 activity tick.

## Implementation Status: 🟡 In Progress (phases 0–7 shipped in v0.46.0; phases 8–13 pending)

**Pending (added 2026-09-12 from the rig-run evidence table, target
v0.47.0):** phase 9 — in-session continuation (D6); phase 10 — threshold
interruptions (D7); phase 11 — compaction-aware inactivity window (D8);
phase 12 — observability (D9); phase 13 — verify, docs, v0.47.0.

#### Follow-on phase log

**Phase 8 (D5) — complete.** `src/adapters/pi_rpc.rs`:

- `RpcFlags` (`agent_end` sticky / `teardown_armed` / `open_compaction`) +
  `TurnState` (`agent_end`, `agent_end_at`, `settled`) and the single
  `teardown_due(turn, compaction_open)` predicate: close stdin on
  `agent_settled`, else on `agent_end` with no span open once
  `SETTLE_WINDOW` (250 ms) has elapsed. The driver loop now reads with
  `recv_timeout(SETTLE_WINDOW)` so the fallback fires while pi is silent.
- The main teardown loop starts `TEARDOWN_GRACE` from `teardown_armed`
  (the driver's decision), so the 5 s grace means "graceful process that
  will not exit" and an in-flight compaction is bounded by the total budget
  only. Module doc + `TEARDOWN_GRACE` doc rewritten (pi exits on stdin EOF).
- **Stdin-aware mock** `pi_like_rpc_mock(log, body)`: pi's stream is emitted
  by a background writer; a foreground loop reads stdin until EOF (logging
  each command plus an `eof` marker) and on EOF **kills the writer and
  exits** — pi's `stdin.on("end", shutdown)`. Verified against the defect:
  an early close loses `compaction_start`/`compaction_end` entirely, a
  settled close captures them.
- Tests: `rpc_agent_end_empty_then_threshold_compaction_completes`,
  `rpc_closes_stdin_on_agent_settled`,
  `rpc_fallback_settle_window_closes_stdin_without_settle_event`, and
  `rpc_fallback_settle_window_defers_for_compaction_start` (the span opened
  inside the settle window still lands). Both D4 tests kept; the older one
  records that its event order is inverted w.r.t. pi.
- **Deviation (implementation detail, no design change):** the main loop
  arms the grace on the driver's `teardown_armed` flag rather than
  re-evaluating the predicate itself — one source of truth for the decision,
  same predicate. `rpc_compaction_attempted_but_could_not_fit` now sets a
  1.2 s timeout: with stdin held open (D5), a mock that stays alive with a
  span open now waits for its total budget instead of exiting at the EOF
  that the old early close used to cause.
- `cargo test` (1063 lib + all integration) and `cargo clippy` — no new
  warnings from this phase (the driver's `too_many_arguments` line now
  carries an `allow`, as `live_output::spawn_watchdog` does).

### Shipped in v0.46.0 (phases 0–7)

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
  ⚠️ **Incomplete in practice:** the hold covers the *force-kill* path only.
  The driver still closes stdin at `agent_end`, and pi's rpc mode exits on
  stdin EOF, so a post-`agent_end` compaction still dies (~1 s in) on every
  threshold compaction — five occurrences on 2026-09-11 (17:10, 19:05,
  22:33, 23:13, 23:27). The D4 tests do not catch it: their mocks never read
  stdin, and one emits `compaction_start` before `agent_end`. Fixed by
  **phase 8**. The first live success of the D2/D3 recovery is also on
  record: 22:47:31 `CompactionStarted{overflow}` → `CompactionInterrupted` →
  `ManualCompactionSucceeded` → `SessionRestarted` → 23:02:57
  `KnotCompleted`.
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
