# Plan 090: Concise In-Session Retry — Stop Re-Sending the Original Prompt on Session Re-Entry

## Related Plans

Changes the *prompt shape* of the session-resume retry loop introduced by
`session-resume-on-invocation-failure.md` and shaped by
[078 Final-Response Request](../078-final-response-request/final-response-request-plan.md)
(the retry prompt is today "original prompt + final-response request").
The cause-specific notes it keeps were added by
[081 Inactivity Timeout](../081-inactivity-timeout/inactivity-timeout-plan.md)
(`INACTIVITY_RESTART_NOTE`),
[086 Graceful Task Handoff](../086-graceful-task-handoff/graceful-task-handoff-plan.md)
(`HANDOFF_NOTE`), and
[089 Interrupted-Compact Manual Recovery](../089-interrupted-compact-manual-recovery/interrupted-compact-manual-recovery-plan.md)
(`COMPACTION_RESTART_NOTE`). Generalises the concise-prompt precedent set by
[059 Tie-Off Event Enforcement](../059-tie-off-event-enforcement/tie-off-event-enforcement-plan.md)'s
`inject_event_request` (follow-up with **no original prompt and no profile
prompt**). Plan 078's note *"Why re-send the original prompt on retry?"* is
the explicit decision this plan reverses.

## Problem

When a session-resume retry re-enters an existing pi session
(`--session-id <id>`), the loop re-sends the **entire composed prompt** —
profile persona + knot/strand prompt + trigger line — plus the
cause-specific note, on **every** attempt. The re-entered session already
holds all of that in its conversation history. Consequences:

1. **Token cost per retry.** Each of the up to 10 retries re-sends the full
   prompt, pure overhead.
2. **Context-pressure amplification.** The in-session retries that matter
   most are the ones fired *because* context ran out (post-compaction
   restart, water-mark handoff, compaction-interrupt recovery). Re-sending
   the full original prompt on exactly those attempts burns the budget the
   compaction/water-mark machinery just spent work to save, and can trigger
   a second compaction of a context that was just shrunk.
3. **Duplication in context.** The agent sees two copies of the task
   instructions (the original turn and the re-sent copy). After a
   compaction the first copy is summarised; the verbatim re-send can
   contradict the summary.

The concise shape is already proven in this codebase: `inject_event_request`
sends a short follow-up with an empty profile prompt ("No profile prompt
needed since the session already has the persona and instructions from the
first turn"), and the plan-089 D6 live continuation sends the bare
`FINAL_RESPONSE_REQUEST` over the running session's RPC channel — both
in-session, both without the original prompt.

## Target

The retry prompt shape is keyed on the one fact that makes conciseness
safe — whether the retry re-enters an existing session:

- **In-session retry** (`session_id` captured → `--session-id` passed):
  the retry prompt is **the cause-specific note only** —
  `FINAL_RESPONSE_REQUEST` (generic), `INACTIVITY_RESTART_NOTE`,
  `COMPACTION_RESTART_NOTE`, or `HANDOFF_NOTE` — with an **empty
  profile prompt**. No accumulation across attempts: each retry prompt is
  exactly the note for that attempt (the notes are already consumed per
  attempt via `pending_note.take()`).
- **Fresh restart** (`session_id` absent — the pre-session inactivity
  stall, the one gate exception from plan 081): **unchanged**. A fresh
  process has no conversation history; the full composed prompt (profile +
  original + note) is the only copy of the task instructions.
- **First attempt**: unchanged (full composed prompt).
- **Already-concise in-session injections — unchanged**: the D6 live
  continuation (plan 089), the wrap-up steer (plan 086), and
  `inject_event_request` (plan 059).
- **`strand_file_ref` keeps being passed on every attempt** — the
  `@{path}` attachment lets the agent re-read the original strand/task file
  on demand, which is the fallback for the (rare) case where compaction
  summarised the original instructions away.
- **Bounds and budget math unchanged**: `MAX_RETRIES = 10`,
  `MIN_REMAINING_SECS = 5`, `RETRY_DELAY = 10s`, profile timeout budget,
  all loom/system events. The notes carry all cause information the agent
  needs; nothing else changes about *when* or *how often* retries happen.

Non-goals:

- No new note text — in particular, no cause-specific
  `TIMEOUT_RESTART_NOTE` for total-timeout overruns (a candidate follow-up:
  the generic `FINAL_RESPONSE_REQUEST` today does not say *why* a
  total-timeout turn was stopped).
- No change to retry *gating*, budget math, events, tie-offs, or
  terminal-error classification.
- No change to adapters (`pi-json` / `pi-rpc` compose and send the prompt
  exactly as given).
- No change to project document formats → no `knot-update` migration entry.

## Existing Tests

| Test | File | What it covers | Status after this plan |
|------|------|----------------|------------------------|
| `retry_appends_final_response_request` | `src/application/session_resume.rs` | Retry prompt contains `FINAL_RESPONSE_REQUEST` | **Must change** — retry prompt is now exactly the note; assert exact-equality and that the original prompt (`"Review this document"`) and profile prompt (`"You are a reviewer."`) are absent |
| `inactivity_retry_prompt_carries_note` (the test asserting `blocked for more than 300 seconds` + `5-minute window` + no 078 text on `contexts[1]`) | `src/application/session_resume.rs` | Inactivity retry note shape | Passes as-is; **extend** with no-original-prompt + empty-profile-prompt assertions |
| `inactivity_note_replaces_final_response_request` | `src/application/session_resume.rs` | Timeout retry still carries the 078 text | Passes as-is; **extend** with no-original-prompt assertion |
| `inactivity_retry_fresh_without_session` | `src/application/session_resume.rs` | Fresh restart (no `--session-id`) carries the note | **Must extend** — becomes the regression guard for the fresh path: full original prompt **and** profile prompt **and** note still present |
| `inject_event_request_does_not_resend_profile_prompt` | `src/application/session_resume.rs` | The concise-prompt precedent (empty profile on follow-up) | Unaffected |
| `retry_succeeds_on_first_retry`, `retry_exhausted_then_fails`, `retry_stops_on_*`, `no_retry_*`, budget/bail tests | `src/application/session_resume.rs` | Retry flow, bounds, budget | Unaffected (no prompt-shape assertions) |
| `process_strand_retry_*` | `src/application/usecases/process_strand.rs` | Retry flows with real `Timeout` | Unaffected |
| `tests/session_resume.rs` integration suite | `tests/session_resume.rs` | End-to-end retry flows via mock ports | Unaffected (asserts tie-offs and `--session-id`, not prompt text) |

## Test Gaps

- No test that an in-session retry prompt is **exactly** the note (no
  original prompt, no profile prompt, no accumulated notes).
- No test that in-session retries do **not** accumulate notes across
  attempts (attempt 2 and attempt 3 prompts are identical).
- No test that the **fresh** restart path still sends the full composed
  prompt + profile prompt + note (the change's main regression risk).

## Phases

### Phase 0: Failing Tests

`src/application/session_resume.rs` unit tests (helpers: `execute` /
`execute_no_budget` use prompt `"Review this document"`, profile prompt
`"You are a reviewer."`):

1. `retry_prompt_is_note_only_in_session` — sequence
   `[Err(err_timeout("sess-abc")), Ok(ok_output("done"))]` via `execute`:
   - `contexts[1].prompt == FINAL_RESPONSE_REQUEST` (exact),
   - `!contexts[1].prompt.contains("Review this document")`,
   - `contexts[1].profile_prompt.is_empty()`,
   - `--session-id` + `sess-abc` present in `extra_args`,
   - first attempt unchanged: `contexts[0].prompt == "Review this
     document"` and `contexts[0].profile_prompt == "You are a
     reviewer."`.
2. `retry_prompts_do_not_accumulate` — sequence
   `[Err(err_timeout("sess-abc")); Err(err_timeout("sess-abc"));
   Ok(ok_output("done"))]`: `contexts[1].prompt == contexts[2].prompt ==
   FINAL_RESPONSE_REQUEST` (each retry is exactly its note; today attempt 3
   carries the original prompt plus both notes).
3. `inactivity_retry_prompt_is_note_only` — in-session inactivity kill:
   `contexts[1].prompt` equals the built
   `inactivity_restart_note(300, 300)` text, `profile_prompt` empty, no
   `"Review this document"`.
4. `fresh_restart_keeps_full_prompt` — `err_inactivity(None)` → fresh
   retry: `contexts[1].prompt` starts with `"Review this document"` **and**
   contains the inactivity note; `contexts[1].profile_prompt == "You are a
   reviewer."`; no `--session-id`. (The guard that keeps plan 081's
   fresh-restart contract intact.)
5. Update `retry_appends_final_response_request` per the Existing-Tests
   note.

### Phase 1: The Concise Retry Prompt

`src/application/session_resume.rs`, `execute_with_resume_internal`, the
retry loop's "Prepare agent_config and prompt for retry" block (today:
`prompt.push_str("\n\n"); prompt.push_str(&note)` and the exec call passing
`prompt.clone(), profile_prompt.clone()`):

```rust
let note = pending_note
    .take()
    .unwrap_or_else(|| FINAL_RESPONSE_REQUEST.to_string());
// Plan 090: key the prompt shape on session re-entry. In-session, the
// conversation already holds the persona, original prompt, and trigger
// line — the note alone is the whole retry prompt (the
// `inject_event_request` precedent). A fresh restart has no history: the
// full composed prompt is the only copy of the task instructions.
let (retry_prompt, retry_profile_prompt) = match session_id {
    Some(_) => (note, String::new()),
    None => {
        prompt.push_str("\n\n");
        prompt.push_str(&note);
        (prompt.clone(), profile_prompt.clone())
    }
};
```

and the `execute_with_config_and_observer` call takes
`retry_prompt` / `retry_profile_prompt` instead of
`prompt.clone()` / `profile_prompt.clone()`.

Notes:

- The `Some` arm drops the accumulation (no `push_str` on the loop-local
  `prompt`), so in-session retries are exactly the per-attempt note. The
  `None` arm preserves today's fresh-restart behaviour byte-for-byte
  (including accumulation, which there is harmless — a fresh restart is
  rare and each note is short).
- `agent_config.extra_args` handling (`--session-id`), `strand_file_ref`,
  timeout computation, and all event logging are untouched.
- Update the module doc comment (lines 6–13: "The retry prompt is the
  original prompt plus the final-response request…") to describe the
  two-shape contract.

### Phase 2: Verify

- `cargo test` — full suite green; watch:
  - the Phase 0 tests (note-only, no-accumulation, fresh-path guard),
  - the plan-081/086/089 note-shape tests (must stay green),
  - `tests/session_resume.rs` and the `process_strand` retry tests
    (untouched flows).
- `cargo clippy --all-targets` clean.

### Phase 3: Docs + Version

1. `docs/concepts.md` — session-resume paragraph: an in-session retry now
   sends only the cause-specific note (the original prompt and profile
   prompt are not re-sent; the `@strand-file` attachment stays available);
   a fresh restart (no session) still sends the full prompt.
2. `.agents/skills/knot-init/knot-glossary.md` — the *Session Resume*
   entry: "Each retry re-sends the original prompt with the final-response
   request appended" → "Each **in-session** retry sends only the
   cause-specific note (the session already holds the original prompt); a
   fresh restart — the pre-session inactivity stall, with no session ID —
   sends the full prompt plus the note."
3. `docs/release-notes.md` — v0.48.0 entry (improvement): concise
   in-session retry prompts; no re-send of the original prompt on
   `--session-id` re-entry; fresh restarts unchanged.
4. `knot-update` skill: **no migration entry** (no document-format
   change).
5. Bump `Cargo.toml` to `0.48.0`; `cargo install --path .`.

## Notes

- **Why this is safe when 078 declined it.** Plan 078 chose re-sending for
  *mechanism consistency* (one prompt shape for all retries). The codebase
  has since grown two proven concise in-session shapes —
  `inject_event_request` (plan 059) and the D6 live continuation
  (plan 089, which sends the bare `FINAL_RESPONSE_REQUEST` over the RPC
  channel and is called out in troubleshooting as "the cheap healthy
  path"). The consistency argument no longer holds: there is already more
  than one shape, and the in-session ones are the short ones.
- **The one case where the agent might want the original text back** — a
  compaction that summarised the original instructions away — is covered by
  the `@{path}` strand-file attachment, which is passed on every attempt
  and unchanged by this plan.
- **Interaction with plan 086's water-mark handoff.** The in-session
  `HANDOFF_NOTE` retry is the main beneficiary: today a water-marked
  re-entry re-sends the full original prompt into a context that just
  crossed the water-mark; after this plan the re-entry carries only the
  handoff note.
- **Terminal-error classification is untouched** — the loop still
  classifies exhaustion by the last failure kind (empty response →
  `AgentNoResponse`, timeout → `Timeout`); only the *prompt text* of each
  attempt changes.
