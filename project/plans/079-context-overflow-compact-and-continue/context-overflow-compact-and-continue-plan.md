# Plan 079: Context Overflow — Compact and Continue, with Loom-Log Visibility

## Related Plans

Builds on [078 Final-Response Request](../078-final-response-request/final-response-request-plan.md)
(the session-resume nudge loop this plan stops mis-firing on) and
[077 Empty Response Is Not a Timeout](../077-empty-response-not-timeout/empty-response-not-timeout-plan.md)
(`PortError::AgentNoResponse`, `TieOffOutcome::derive` semantics).
Interacts with `session-resume-on-invocation-failure.md` (retry loop,
`--session-id` re-entry, budget tracking) and pairs with
[051 Pi JSON Final Response Filter](../051-pi-json-final-response-filter/pi-json-final-response-filter-plan.md)
— the filter that drops `stopReason: "error"` messages, which is what
makes a context overflow *look* like plan 078's "empty response".

## Problem

**Observed:** when a knot's pi session hits the model's context limit,
the session-resume retry loop burns all 10 retries — each with its
10-second delay and a full attempt — before the strand fails with
`no final response: agent returned empty response after 11 attempts
(session resume exhausted)`. Every retry clocks up; none can succeed.

**Root cause chain** (verified against the installed pi 0.78.x source):

1. `~/.pi/agent/settings.json` sets `"compaction": { "enabled": false }`
   — pi's built-in compaction is disabled *entirely*: `_checkCompaction()`
   in `agent-session.js` returns immediately when the setting is off.
2. With compaction off, an over-full context surfaces as the LLM's
   overflow error; the turn ends with an assistant message
   `stopReason: "error"`.
3. `pi --mode json` exits **0** in that case — the non-zero exit path
   exists only for `mode === "text"` (`dist/modes/print-mode.js`) — so
   knot sees a *successful* invocation.
4. Plan 051's final-response filter drops `stopReason: "error"`
   messages → empty stdout → plan 078's nudge loop re-enters the *same
   over-full session* via `--session-id` → each retry overflows again →
   10 retries × (10 s + attempt time) clock up → `AgentNoResponse`.

**pi already ships compact-and-continue** — it is simply disabled:

- **Threshold** — proactive compaction when
  `contextTokens > contextWindow − reserveTokens` (default 16,384
  reserved), so the hard limit is rarely reached at all.
- **Overflow** — `isContextOverflow()` (pi-ai `utils/overflow.js`,
  ~20 provider error patterns) matches the error; pi removes the error
  message, compacts, and **auto-retries the prompt in-process**
  (`compaction_end { reason: "overflow", willRetry: true }`). Recovery
  is once per user message; the flag resets on each new user message
  and each non-error assistant message — so every knot session-resume
  (a new user message) gets a fresh recovery chance.
- **Observable** — both paths emit `compaction_start` / `compaction_end`
  on the JSON stream knot already consumes (json mode writes *every*
  session event to stdout).
- **Unrecoverable** — if the kept context (default 20k recent tokens)
  itself cannot fit, pi emits
  `compaction_end { reason: "overflow", willRetry: false,
  errorMessage: "Context overflow recovery failed after one
  compact-and-retry attempt…" }` and the turn ends in error.

Knot cannot drive compaction itself: in non-interactive modes
`/compact` is **not** a parsed command (only extension/skill/
prompt-template commands expand in `session.prompt()`), and the RPC
`compact` command would require a long-lived RPC process — out of
scope for this plan.

## Target

1. **Enable compaction for rig sessions** via a project-level
   `.pi/settings.json` (project settings override global; knot spawns
   pi inheriting the rig project's CWD):

   ```json
   { "compaction": { "enabled": true } }
   ```

   - This repo: commit the file.
   - Other rigs: the `knot-init` skill seeds it at rig initialisation
     (create-if-absent, never overwrites).
2. **Loom-log visibility** — a new `LoomEvent::ContextCompacted`, one
   entry per compaction observed in an invocation's stream (the
   "context reached, compacting" record). Purpose: spot which
   knot/strand hits context pressure so the prompt and strand scope
   can be narrowed.
3. **Fail fast on unrecoverable overflow** — a new non-resumable
   `PortError::ContextLimitReached`: no session-resume retries,
   `Failed` tie-off with pi's message, no rig-log timeout.
4. **Continue up to the timeout limit** — no new budget mechanism:
   compaction happens inside pi within the existing per-attempt
   timeout (`budget − elapsed`) and the loop's `MIN_REMAINING_SECS`
   bail. The profile timeout still caps strand wall time.

Behavioural contract:

- **Config** — `.pi/settings.json` at the rig project root with
  `{"compaction": {"enabled": true}}`. Default `reserveTokens` /
  `keepRecentTokens` (16k / 20k) are safe for the current local models
  (120k–200k windows); small-window models need tuning (Notes).
- **Parsing** — `PiJsonAgentRunner` records every `compaction_end`
  event in stream order as
  `CompactionRecord { reason, tokens_before, will_retry, error }` on
  `AgentInvocationMetadata.compactions`. `compaction_start` is ignored
  (the end event carries everything).
- **Logging** — `execute_with_resume` appends `ContextCompacted` for
  each **successful** compaction (`error == None`) from both the first
  attempt and the retries; `attempt` follows the `KnotEmptyResponse`
  convention (1 = first attempt, 2 = first retry). The append is
  best-effort (`let _`) — observability must never fail a strand.
- **Fail-fast detection** — in `PiJsonAgentRunner::execute`, on the
  clean-parse path: the response is empty **and** the stream contains
  a `compaction_end` with `reason: "overflow"` + `willRetry: false`
  **preceded by** one with `reason: "overflow"` + `willRetry: true` in
  the same invocation (recovery ran and the context still does not
  fit) ⇒ `Err(PortError::ContextLimitReached { message, session_id })`.
  - `message` = the failing record's pi `errorMessage` when present,
    else `session context cannot fit the model window even after
    compaction`.
  - A `willRetry: false` overflow **without** a preceding
    `willRetry: true` (compaction could not even run — missing
    model/auth, transient summarisation API error) is **not**
    fail-fast: it falls into the existing nudge loop, where the new
    user message gives pi a fresh recovery attempt.
- **Error semantics** — `ContextLimitReached` is **not resumable**
  (`is_resumable()` returns false): `execute_with_resume` returns it
  immediately from the first-attempt path or exits the loop from the
  in-loop path — no `SessionResumed`, no clock-up.
  `TieOffOutcome::derive`'s catch-all maps it to a `Failed` tie-off +
  `KnotFailed` loom event; **no** rig-log timeout (no deadline was
  exceeded — the same class as plan 077).

Non-goals:

- No long-lived RPC pi process; no knot-driven `/compact`.
- No copy of pi's overflow regex patterns into knot (detection is
  structural, via pi's own compaction events).
- No new profile/knot configuration knobs.
- Compactions during the event-enforcement follow-up
  (`inject_event_request`) are not logged (rare; that path already
  logs `KnotEventsMissing`).
- Silent overflow (providers that accept over-full requests without an
  error) is not detected by knot — pi's own `isContextOverflow` covers
  recovery; knot sees only the outcome.
- No rig document format changes → no `knot-update` migration (a
  changelog entry only).

## Existing Tests

| Test | File | What it covers | Status after this plan |
|------|------|----------------|------------------------|
| pi-json suite (`json_l_parsing_*`, `stop_reason_*`) | `tests/adapters.rs` | JSON-L parsing, stopReason filter | Unaffected — `stop_reason_error_excluded` still passes: no compaction events ⇒ empty `Ok`, the nudge loop keeps its job |
| `parse_stdout` unit tests | `src/adapters/pi_json.rs` | Parser | `parse_stdout` gains a `compactions` out-param — tests move to the 5-tuple |
| `AgentInvocationMetadata` constructions | `src/adapters/pi_json.rs` (×1 prod), `src/application/usecases/process_strand.rs` (×7), `src/application/session_resume.rs` (×1 test helper), `tests/session_resume.rs` (×1), `tests/event_enforcement.rs` (×1) | — | Mechanical: add `compactions: vec![]` |
| session-resume unit suite | `src/application/session_resume.rs` | Retry loop | Unaffected (metadata helper updated) |
| `TieOffOutcome::derive` tests | `src/domain/entities.rs` | Outcome mapping | Unaffected (catch-all) — one new variant test added |
| `PortError` `Display` / `session_id()` / `is_resumable()` | `src/application/ports.rs` | Error classification | New variant arms added; `is_resumable()` deliberately excludes the new variant |

## Test Gaps

- No parsing of `compaction_end` (success or failure shape).
- No terminal-overflow ⇒ `ContextLimitReached` classification
  (adapter level, mock binary).
- No `ContextCompacted` loom-log emission tests (first attempt, retry,
  failed-compaction exclusion).
- No non-resumable-gate test for the new error in the retry loop.
- No `Failed`-tie-off / no-`SessionResumed` / no-rig-log test for
  terminal overflow at `process_strand` level.

## Phases

### Phase 0: Failing Tests

1. `src/application/ports.rs` — unit tests:
   - `context_limit_reached_not_resumable` —
     `PortError::ContextLimitReached { … }.is_resumable()` is `false`;
     `session_id()` returns the carried id; `Display` contains
     `context limit reached`.
2. `src/adapters/pi_json.rs` — parser unit tests (direct
   `parse_stdout`, 5-tuple):
   - `test_json_runner_parses_compaction_end` — raw: session +
     `{"type":"compaction_end","reason":"overflow","result":{"summary":"s","firstKeptEntryId":"e","tokensBefore":150000,"details":{}},"aborted":false,"willRetry":true}`
     + `agent_end` (stop, "done"); expect record
     `{ reason: "overflow", tokens_before: Some(150000),
     will_retry: true, error: None }`.
   - `test_json_runner_parses_compaction_end_failed` — raw: session +
     `{"type":"compaction_end","reason":"overflow","aborted":false,"willRetry":false,"errorMessage":"Context overflow recovery failed after one compact-and-retry attempt. …"}`
     + `agent_end` (error message only); expect record
     `{ will_retry: false, tokens_before: None, error: Some(…) }` and
     empty response text.
   - `test_json_runner_ignores_compaction_start` — a
     `compaction_start` line produces no record.
   Mock-binary tests (existing bash-script pattern — `cat > /dev/null`
   + `echo` JSON lines + `exit 0`):
   - `test_json_runner_terminal_overflow_returns_context_limit_reached`
     — script emits, in order: session (id `sess-ctx`),
     `compaction_end { overflow, willRetry: true, result
     { tokensBefore: 150000 } }`,
     `compaction_end { overflow, willRetry: false, errorMessage }`,
     `agent_end` (error message only); expect
     `Err(PortError::ContextLimitReached)`; `session_id()` ==
     `Some("sess-ctx")`; message contains the pi errorMessage text.
   - `test_json_runner_overflow_recovered_is_success` — script emits:
     session, `compaction_end { overflow, willRetry: true, result }`,
     `agent_end` (stop, "done after compact"); expect `Ok`, stdout
     contains `done after compact`, `metadata.compactions` holds one
     record.
3. `tests/adapters.rs` — adapter-level (mock binary):
   - `overflow_without_prior_recovery_is_not_fail_fast` — script emits:
     session, `compaction_end { overflow, willRetry: false,
     errorMessage }` (no `willRetry: true`), `agent_end` (error only);
     expect `Ok` with **empty** stdout — the nudge loop keeps its job
     for compaction failures that never ran.
4. `src/application/session_resume.rs` — unit tests (mock runner
   sequence; `ok_output` helper extended with a compactions parameter
   or a sibling helper):
   - `compaction_logged_on_first_attempt` — runner
     `[Ok(output("done", sid, compactions = [rec(overflow,
     tokens_before: 150000, will_retry: true)]))]`; expect `Ok`;
     loom-log: `ContextCompacted { reason: "overflow",
     tokens_before: Some(150000), attempt: 1 }`; **no**
     `SessionResumed`.
   - `compaction_logged_on_retry_attempt` — runner
     `[Err(err_timeout(sid)), Ok(output("done", sid, compactions =
     [rec(threshold)]))]`; expect `Ok`; loom-log:
     `SessionResumed { attempt: 1 }` then `ContextCompacted
     { reason: "threshold", attempt: 2 }`.
   - `failed_compaction_not_logged` — first-attempt `Ok` with
     `compactions = [rec(overflow, error: Some("…"), will_retry:
     false)]` and non-empty stdout; expect `Ok` and **no**
     `ContextCompacted`.
   - `terminal_overflow_first_attempt_no_retry` — runner
     `[Err(err_context_limit(sid))]`; expect immediate
     `Err(PortError::ContextLimitReached)`; runner called **once**;
     loom-log empty (no `SessionResumed`).
   - `terminal_overflow_in_loop_exits` — runner
     `[Err(err_timeout(sid)), Err(err_context_limit(sid))]`; expect
     `Err(PortError::ContextLimitReached)`; loom-log:
     `SessionResumed { attempt: 1 }` only; runner called twice.
5. `src/application/usecases/process_strand.rs` — `execution_tests`
   (zero retry delay):
   - `process_strand_terminal_overflow_failed_tieoff` — runner
     `[Err(err_context_limit(sid))]`; expect: tie-off appended
     **Failed** with content containing `context limit reached`;
     loom-log: `KnotProcessing`, `KnotFailed`,
     `StrandProcessed { error: Some(…) }`; rig-log **empty**; no
     `SessionResumed`, no `KnotEmptyResponse`.
   - `process_strand_compaction_visible_on_success` — runner
     `[Ok(output("done", sid, compactions = [rec(overflow)]))]`;
     expect: tie-off appended **Produced**; loom-log contains
     `ContextCompacted { attempt: 1 }` and `KnotCompleted`.
6. `src/domain/entities.rs`:
   - `derive_context_limit_reached_is_failed` —
     `TieOffOutcome::derive(Err(ContextLimitReached { … }))` ⇒
     `Failed` (not `TimeoutSkipped`), `should_write_tie_off()` true,
     `is_timeout()` false.

### Phase 1: Compaction Parsing and Metadata

`src/application/ports.rs`:

- New value type:
  ```rust
  /// One `compaction_end` event observed in the agent's JSON stream.
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct CompactionRecord {
      /// pi's compaction reason: `"overflow"` (context limit hit),
      /// `"threshold"` (proactive), or `"manual"`.
      pub reason: String,
      /// Context tokens before compaction
      /// (`compaction_end.result.tokensBefore`); `None` when the
      /// compaction failed (no result).
      pub tokens_before: Option<u64>,
      /// True when pi auto-retries the prompt after compaction.
      pub will_retry: bool,
      /// pi's error message when compaction failed (`errorMessage`).
      pub error: Option<String>,
  }
  ```
- `AgentInvocationMetadata` gains
  `pub compactions: Vec<CompactionRecord>` with `#[serde(default)]`;
  doc comment updated.
- Update every construction site listed in the Existing Tests table.

`src/adapters/pi_json.rs`:

- `parse_json_line` / `parse_stdout` thread a
  `compactions: &mut Vec<CompactionRecord>` through; new arm:
  ```rust
  Some("compaction_end") => {
      compactions.push(CompactionRecord {
          reason: value.get("reason")
              .and_then(|r| r.as_str())
              .unwrap_or_default()
              .to_string(),
          tokens_before: value.get("result")
              .and_then(|r| r.get("tokensBefore"))
              .and_then(|t| t.as_u64()),
          will_retry: value.get("willRetry")
              .and_then(|b| b.as_bool())
              .unwrap_or(false),
          error: value.get("errorMessage")
              .and_then(|e| e.as_str())
              .map(String::from),
      });
  }
  ```
  (`result` is absent on failed compactions — pi omits undefined keys
  in JSON — so `tokens_before` is `None` there.)
- `execute()` builds the metadata with the collected records.

### Phase 2: Terminal-Overflow Fail-Fast

`src/application/ports.rs`:

- New variant:
  ```rust
  /// The session's context exceeds the model window and pi's own
  /// compact-and-retry could not recover it — the kept context
  /// itself cannot fit. Terminal: session-resume re-entry cannot
  /// help, so this is NOT resumable.
  ContextLimitReached {
      message: String,
      session_id: Option<String>,
  },
  ```
  - `Display`: `context limit reached: {message}`
  - `session_id()`: joins the `Some` arm.
  - `is_resumable()`: **not** added (deliberate); the doc comment
    names the terminal context condition.

`src/adapters/pi_json.rs`, `execute()` — on the clean-parse success
path, before returning `Ok`:

- Helper:
  ```rust
  /// The failing record when the stream shows a terminal overflow:
  /// an overflow compaction with `will_retry: false` that follows an
  /// overflow compaction with `will_retry: true` (recovery ran, the
  /// context still does not fit).
  fn terminal_overflow(records: &[CompactionRecord]) -> Option<&CompactionRecord>
  ```
- If `response_text.trim().is_empty()` and `terminal_overflow(&compactions)`
  is `Some(rec)` ⇒
  `return Err(PortError::ContextLimitReached { message, session_id })`,
  where `message` is `rec.error` when present, else `session context
  cannot fit the model window even after compaction`.

### Phase 3: Loom-Log `ContextCompacted`

`src/domain/events.rs`:

- New variant:
  ```rust
  /// The agent session's context hit (or approached) the model window
  /// and pi compacted it. One entry per compaction observed in an
  /// invocation's JSON stream. `reason` is `"overflow"` (the context
  /// limit was hit — compacted to continue) or `"threshold"` (pi
  /// proactively compacted before the limit). The entry marks context
  /// pressure so the prompt/strand scope can be narrowed.
  ContextCompacted {
      loom_id: LoomId,
      knot_id: KnotId,
      strand_path: StrandPath,
      session_id: String,
      reason: String,
      tokens_before: Option<u64>,
      /// Attempt the compaction was observed on
      /// (1 = first attempt, 2 = first retry, …).
      attempt: u32,
      timestamp: String,
  },
  ```
- Loom-log reader: no change — old binaries reading a line with the
  new variant skip it with a warning (the 0.33.0
  `EventsDispatched`-tuple precedent); new binaries read old logs
  unchanged.

`src/application/session_resume.rs`, `execute_with_resume_internal`:

- Helper:
  ```rust
  /// Append one `ContextCompacted` loom event per successful
  /// compaction in the invocation's metadata. Best-effort:
  /// observability must never fail a strand.
  fn log_compactions(
      loom_log: &dyn LoomLogPort,
      loom_id: &LoomId,
      knot_id: &KnotId,
      strand_path: &StrandPath,
      attempt: u32,
      output: &AgentOutput,
  )
  ```
  — for each record with `error.is_none()`,
  `let _ = loom_log.append(LoomEvent::ContextCompacted { …,
  session_id: metadata.session_id.clone().unwrap_or_default(), … })`.
- Call sites: the first-attempt success branch (before `return
  Ok(output)`, `attempt: 1`) and the in-loop success branch (before
  `return Ok(output)`, `attempt: attempt + 1`) — the
  `KnotEmptyResponse` attempt convention.

### Phase 4: Rig Configuration — Enable Compaction

1. This repo: commit `.pi/settings.json`:
   ```json
   { "compaction": { "enabled": true } }
   ```
   Project-level override of the global `enabled: false`; it does not
   affect interactive pi outside this directory.
2. `.agents/skills/knot-init/SKILL.md`: add a step to the
   initialisation flow — create `.pi/settings.json` with the
   compaction block **only if the file does not exist** (never
   overwrite an existing settings file); verify by reading it back.
   Deploy the updated skill per AGENTS.md (copy to
   `~/.agents/skills-library/knot-init/` and diff-verify).

### Phase 5: Verify + Regression

- `cargo test` — full suite green; watch in particular the pi-json
  parser/mock suites, the `session_resume` unit tests, the
  `process_strand` execution tests, `tests/adapters.rs`,
  `tests/session_resume.rs`, and `tests/pipeline.rs`.
- `cargo clippy --all-targets` clean.
- Manual rig check (optional): a long knot run that compacts — or a
  stub pi that emits `compaction_end` JSONL — verify the loom-log
  shows `ContextCompacted` entries and the tie-off is `Produced`.

### Phase 6: Docs + Version

1. `docs/concepts.md` — session section: pi's built-in compaction is
   enabled for rig sessions (`.pi/settings.json`);
   `ContextCompacted` loom entries mark context pressure
   (`overflow` = limit hit, `threshold` = proactive); terminal
   overflow fails the strand immediately (no retry clock-up).
2. `docs/troubleshooting.md` — new row:
   *"KnotFailed: context limit reached"* — pi compacted the session
   and the context still does not fit. Narrow the prompt/strand scope
   (smaller strand files, tighter knot instructions, `@file`
   references instead of inlined content) or use a profile with a
   larger-window model. Check `ContextCompacted` entries for
   frequency.
3. `docs/release-notes.md` — v0.38.0 entry (feature):
   compact-and-continue via pi's built-in compaction;
   `ContextCompacted` loom-log visibility; `ContextLimitReached`
   fail-fast; config note (`.pi/settings.json` seeded by knot-init).
4. `knot-update` skill: changelog entry (Knot 0.38.0) — **no migration
   required** (no profile/knot/loom/tie-off format change; the new
   loom-log event variant degrades gracefully for older readers;
   `.pi/settings.json` is created only if absent). Deploy per
   AGENTS.md.
5. Bump `Cargo.toml` to `0.38.0`; `cargo install --path .`.

## Notes

- **Why project-level settings, not global:** the global
  `~/.pi/agent/settings.json` explicitly sets
  `compaction.enabled: false` — an operator choice for interactive
  use. Pi resolves project settings (`.pi/settings.json`) over
  global, and knot spawns pi inheriting the rig project's CWD, so the
  change scopes to rig sessions in that directory. knot-init seeding
  makes it uniform across rigs without touching any user's global
  file.
- **Why structural detection (compaction events), not regex:** pi
  maintains ~20 provider overflow patterns; duplicating them in Rust
  would drift. The `compaction_end` event is pi's own classification
  of "this was a context overflow" — knot only asks "did recovery run
  and still fail?".
- **Why the fail-fast needs the `willRetry: true → false` pair:** a
  `willRetry: false` overflow *without* a preceding successful
  compaction means compaction could not run (missing model/auth,
  transient summarisation API error) — a nudge retry (new user message
  ⇒ fresh recovery attempt) is still the right move there. The pair
  (recovered once, then overflowed again) means the kept context
  itself does not fit; no re-entry can help, and 10 retries ×
  (10 s + attempt) is pure clock — the exact "all retries clock up"
  symptom, removed at the cause.
- **Budget / "continue up to the timeout limit":** compaction happens
  inside pi within a single knot attempt. The per-attempt timeout is
  already `budget − elapsed` and the loop bails at
  `MIN_REMAINING_SECS` — nothing new to bound; the profile timeout
  still caps strand wall time.
- **Loom-log semantics for the operator:** `reason: "overflow"` = the
  context limit was hit (the entries to count when narrowing prompt
  scope); `reason: "threshold"` = pi proactively compacted before the
  limit (informational). `tokens_before` shows the pre-compaction
  size.
- **Small-window models:** the defaults (`reserveTokens` 16,384,
  `keepRecentTokens` 20,000) assume a window well above 20k. For a
  model whose window is close to `keepRecentTokens`, set
  `compaction.keepRecentTokens` (and possibly `reserveTokens`) in the
  project `.pi/settings.json` so the kept context fits — otherwise
  every overflow becomes terminal. The current local models declare
  120k–200k windows, so the defaults are safe here.
- **Interaction with plan 078:** the nudge loop is unchanged for
  every other failure shape. Terminal overflow simply never enters it
  (non-resumable error); recovered overflows never reach it (the
  in-process retry yields a normal final response).
