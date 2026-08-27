# Plan 080: Context Overflow Without Compaction — Fail Fast, Warn at Startup

## Related Plans

Follow-up to [079 Context Overflow — Compact and Continue](../079-context-overflow-compact-and-continue/context-overflow-compact-and-continue-plan.md)
(the plan this one closes a gap in) and
[078 Final-Response Request](../078-final-response-request/final-response-request-plan.md)
(the nudge loop that the gap fed).

## Problem

**Observed 2026-08-27 in the `borrow-my-stuff` rig** — the exact failure
plan 079 set out to stop, despite plan 079 being complete:

```
[KNOT][STRAND] Modified failed (knot=phase-implementer): no final response:
agent returned empty response after 11 attempts (session resume exhausted)
— …/coding-implementation-loom/PhaseReady/event-2026-08-27T14-59-10+10-00.md
```

Loom-log: `KnotProcessing` → `KnotEmptyResponse(1)` → 10 ×
(`SessionResumed` + `KnotEmptyResponse`) → `KnotFailed`. No
`ContextCompacted` events at all. The pi session file
(`01a041b0-…`) shows every turn ending

```
stopReason: "error"
errorMessage: "400 request (200287 tokens) exceeds the available context
size (200192 tokens), try increasing it"
```

with the token count *growing* on each nudge (200,287 → 216,172) — each
retry re-sent the profile prompt and re-attached the event file, making
the over-full session fuller. None of the 11 attempts could succeed.

## Root Cause

Plan 079's fix had two assumptions that did not both hold:

1. **Compaction enabled for rig sessions.** Phase 4 seeded
   `.pi/settings.json` (`{"compaction": {"enabled": true}}`) into this
   repo and taught `knot-init` (4.6.0) to seed it for **new** rigs,
   create-if-absent. The `borrow-my-stuff` rig was initialised **before**
   that, and its global `~/.pi/agent/settings.json` sets
   `compaction.enabled: false` — so pi's auto-compaction was off for
   this rig entirely.
2. **Overflow is structurally observable.** With compaction off, pi's
   `_checkCompaction()` returns immediately: no overflow recovery, no
   `compaction_end` events. The overflow surfaces **only** as the
   provider's 400 on the `stopReason: "error"` message. Plan 079's
   fail-fast (`terminal_overflow`) keys off `compaction_end` records —
   it finds nothing, the response text is empty (plan 051's filter
   drops error messages), and the failure degenerates into plan 078's
   nudge loop burning all 11 attempts.

Verified live: forking the failed session and running one turn through
`pi --mode json` reproduces the stream exactly —
`agent_end.messages[]` carries the assistant message with
`stopReason: "error"` and the `errorMessage` field, and no
`compaction_end` line.

## Target

1. **Fail fast on the provider overflow error** — when the response is
   empty and the turn's error message matches a context-overflow
   signature, return `PortError::ContextLimitReached` (non-resumable,
   plan 079 semantics): no nudge loop, no clock-up, a `KnotFailed`
   whose message carries the provider error and the settings fix.
2. **Warn at startup when compaction resolves to disabled** for the
   project, so the gap is visible before a strand hits the wall.
3. **Remediate existing rigs** — one-off creation of
   `.pi/settings.json` per pre-4.6.0 rig (create-if-absent, exactly
   what `knot-init` 4.6.0 does). Applied to `borrow-my-stuff` with this
   plan; other rigs get it on their next `knot-init` re-run (idempotent)
   or manually.

Behavioural contract:

- **Parsing** — `parse_stdout` additionally returns the failed turn's
  provider error message: the `errorMessage` of the
  `stopReason: "error"` assistant message(s) in `agent_end` (last one
  wins). `compaction_end` `errorMessage`s are unaffected (they stay on
  the `CompactionRecord`).
- **Classification** — `PiJsonAgentRunner::is_context_overflow_message`
  matches a conservative set of case-insensitive substrings mirroring
  the common cases of pi's `OVERFLOW_PATTERNS` (pi-ai
  `utils/overflow.js`): llama.cpp "exceeds the available context
  size", Anthropic "prompt is too long" / `request_too_large`, OpenAI
  "exceeds the context window" / "maximum context length", Google
  "exceeds the maximum number of tokens", xAI, Groq, OpenRouter/
  Poolside, Together, Copilot, LM Studio, MiniMax, Kimi, z.ai, and the
  generic "too many tokens" / "context length exceeded" / "token limit
  exceeded" fallbacks. Bedrock-style throttling ("Too many tokens,
  please wait…") is excluded, as in pi's `NON_OVERFLOW_PATTERNS`
  ("rate limit", "too many requests", "throttl").
- **Fail-fast** — in `PiJsonAgentRunner::execute`, clean-parse path:
  response empty **and**
  (a) a plan-079 terminal `compaction_end` record exists, **or**
  (b) the captured error message classifies as overflow ⇒
  `Err(PortError::ContextLimitReached)`. Case (b)'s message:
  `context overflow, but pi auto-compaction did not run (<provider
  error>). Enable compaction for rig sessions with a project-level
  .pi/settings.json: {"compaction": {"enabled": true}}`.
  Everything else (non-overflow error messages, e.g. rate limits)
  keeps today's behaviour — empty `Ok` response, nudge loop retries,
  because transient errors can recover.
- **Startup warning** — `run_startup` (service and step modes) resolves
  the effective compaction setting the same way pi does — project
  `.pi/settings.json` over global `~/.pi/agent/settings.json`, pi's
  default `enabled` when neither sets the key — and prints a WARNING
  naming the project settings file when it resolves to disabled.
  Non-fatal; settings stay the operator's.
- **Why fail-fast is safe here** — within one strand the model is
  fixed, so an overflow error means the context cannot fit *that*
  model, and every nudge adds tokens. There is no retry that can
  succeed while compaction is off. (With compaction on, case (a) / the
  `compaction_end` flow already handles it, or pi recovers in-process.)

Non-goals:

- No knot-side compaction driving (plan 079 non-goal stands: no RPC,
  no `/compact`).
- No knot-written settings files at runtime — seeding stays with
  `knot-init`; knot only warns.
- No changes to the nudge loop itself (plans 077/078 semantics intact).
- No rig document format changes → `knot-update` changelog entry only.

## Changes

| File | Change |
|------|--------|
| `src/adapters/pi_json.rs` | `parse_json_line`/`parse_stdout` capture `errorMessage` of `stopReason:"error"` assistant messages (6-tuple); `is_context_overflow_message`; fail-fast branch in `execute`; `read_pi_compaction_enabled` / `effective_pi_compaction_enabled` settings resolvers |
| `src/server.rs` | `warn_if_pi_compaction_disabled(rig_dir)` called from `run_startup` |
| `Cargo.toml` | version 0.38.0 → 0.38.1 |
| `.agents/skills/knot-update/SKILL.md` | 0.38.1 changelog entry (behavioural, no migration) |
| `~/workspace/borrow-my-stuff/.pi/settings.json` | created (one-off rig remediation, create-if-absent) |

## Tests

- `test_json_runner_parses_error_message` — `parse_stdout` captures the
  overflow `errorMessage`; error stopReason still excluded from
  response text.
- `test_json_runner_no_error_message_on_success` — clean turn leaves
  the capture `None`.
- `test_json_runner_overflow_error_without_compaction_is_context_limit`
  — mock binary emitting the exact borrow-my-stuff stream (session +
  `agent_end` with the llama.cpp 400, no compaction events) ⇒
  `ContextLimitReached`, session ID carried, message carries the
  provider error and the settings hint.
- `test_json_runner_non_overflow_error_without_compaction_stays_ok` —
  rate-limit error message ⇒ `Ok` with empty response (nudge loop
  keeps its job).
- `test_is_context_overflow_message_classification` — positive
  signatures (llama.cpp, Anthropic, OpenAI, LiteLLM, Gemini, 413) and
  negatives (429 rate limit, Bedrock throttling with "Too many
  tokens", 401, 500, empty).
- `test_effective_pi_compaction_enabled_resolution` — both files absent
  ⇒ default enabled; global disabled + project absent ⇒ disabled (the
  incident shape); project enabled overrides global disabled; project
  file without the key / unparseable ⇒ falls through to global;
  explicit project disabled honoured.
- Pre-existing plan 079/078 suites unaffected (full suite green:
  904 + 19 + 34 tests).

## Phases

All phases complete 2026-08-27 (implemented interactively from the
incident; tests written alongside):

1. Rig remediation — `.pi/settings.json` created in
   `borrow-my-stuff` (verified: parses, contains the compaction block).
2. Adapter — error-message capture, classifier, fail-fast branch.
3. Startup warning — settings resolution + WARNING in `run_startup`.
4. Tests — new unit/integration tests above; full suite green.
5. Docs + release — this plan, master index, `knot-update` 0.38.1
   entry, `Cargo.toml` bump.

## Notes

- **Re-dispatching the failed strand** — the failed `PhaseReady` event
  file is still in place; touching it re-triggers `phase-implementer`
  with a *fresh* pi session (session IDs are not persisted across
  strand runs). The new session re-reads the plan checklist on disk, so
  the partially completed phase is picked up where the files left off
  (knot idempotency). With compaction now enabled, the session also
  auto-recovers from overflow in-process.
- **Other pre-4.6.0 rigs** — the startup warning now names the fix at
  every startup; re-running `knot-init` in a project seeds the file
  idempotently.
- **Classifier drift** — if a new provider's overflow wording is not in
  the substring set, the failure falls back to the (harmless but slow)
  nudge loop and the `KnotFailed` message still shows the provider
  error, making the addition obvious.
