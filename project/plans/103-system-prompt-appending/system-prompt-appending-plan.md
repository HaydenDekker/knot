# Plan: System-Prompt Appending — Standing Instructions Move Out of the User Prompt

## Implementation Status: ✅ Complete (2026-11-02)

All six phases landed on `refactor/system-prompt-appending`, released in
**v0.66.0**. Full suite green (1561 tests, 0 failures), `cargo clippy
--all-targets` clean (0 errors).

**What shipped:** the `# Tie-Off` block, `# Subscriber Events` block,
and profile persona are delivered via `--append-system-prompt` (all
three adapters) so they survive compaction; the user message carries
only runtime data. Each run mints a session id and freezes the composed
system prompt to `tie-offs/<rig>/system-prompts/<session-id>.md`; the
artifact rides the spawn args so retries re-supply it verbatim. Drift is
surfaced (`PROMPT-DRIFT`), never silently applied mid-session. New
service records: `PromptFrozen`, `PROMPT-DRIFT`.

**Skill ripple (done post-landing):** `knot-template` 3.1.0,
`knot-create` 5.12.0, `knot-dispatch` 2.1.0, `knot-init` 4.14.0 +
glossary, and the `knot-update` 0.66.0 changelog entry — all deployed to
`~/.agents`. The pirate-probe test skill was intentionally **not**
created (per instruction).

## Problem

The standing instructions are delivered in **user space**. Today every agent
invocation builds one user-message string: profile prompt → knot
instructions (which carry the `# Tie-Off` block and the consumer-gated
`# Subscriber Events` block) → trigger line
(`PiStdioAgentRunner::build_prompt_with_context`, mirrored in `pi_json.rs`
and `pi_rpc.rs`), pushed to pi over stdin. Three consequences:

1. **Compaction eats the contract.** pi's compaction summarizes everything
   before the cut point in the transcript; the system prompt is rebuilt
   untouched each request. A long-running knot's tie-off contract and event
   guide get summarized away mid-session — the agent can drift from the
   contract it was given (verified empirically against pi 0.85.x).
2. **Instruction/data confusion.** The tie-off contract, the event
   content guide, and the profile persona are standing identity and
   contract — not conversational turns. In user space they masquerade as
   something the user just said.
3. **The task is buried.** The only genuinely per-invocation runtime data
   — the knot's task direction, the trigger event, the pending-events
   list — sits behind kilobytes of identical boilerplate.

A fourth, operational problem drove the design: **pi never persists the
system prompt.** The session JSONL records only the message tree
(header: `type/version/id/timestamp/cwd`; entry types: `session`,
`model_change`, `thinking_level_change`, `message`). Verified
empirically: the `--append-system-prompt` string appears nowhere in the
session file. So moving instructions to the system prompt removes
today's accidental observability (the first user message as a config
record) and requires a deliberate durable record.

## Target

After this change:

- **System prompt (via `pi --append-system-prompt`, applied at spawn,
  all three adapters)** carries the standing instructions, in this order:
  1. the `# Tie-Off` block (hard-coded contract + overridable tie-off
     content section — every knot, every new session),
  2. the `# Subscriber Events` block (contract + content guide — only
     when the knot has subscribers / self-continuation; same gating as
     today),
  3. the profile prompt (persona), last.
- **User space (stdin)** carries only runtime data: the knot's task
  instructions, the trigger line (`**<knot>** triggered by **<event>**
  on **<strand>**`), and the pending-events section.
- **The frozen prompt artifact**: at spawn, Knot writes the fully
  rendered system prompt to `tie-offs/<rig>/prompts/<session-id>.md`
  and emits a service-log record binding the run
  (`[KNOT][STATE] type=PROMPT-FROZEN session=<id> knot=<id> file=...`).
  This is the durable answer to "what config did this invocation run
  under?" — pi will not keep one.
- **Resume re-supplies the frozen artifact**: `session_resume.rs` passes
  the frozen file as `--append-system-prompt` and diffs the freshly
  rendered prompt against it, logging
  `type=PROMPT-DRIFT` when the rig changed underneath the session.
- The per-new-session fresh-pull semantics of the instruction files are
  unchanged (render happens at spawn; nothing cached).

Headline property: **the tie-off contract survives compaction**, because
it is no longer in the transcript.

## Existing Tests

| Test Class | What it covers | Status |
|------------|----------------|--------|
| `agent_config_build_cli_args_no_system_prompt_flag` (`value_objects.rs`) | Asserts `build_cli_args` emits **no** system-prompt flag | ⚠️ Asserts the *old* invariant — must be inverted in Phase 5 |
| `profile_ref_cli_args_no_system_prompt_flag` (`process_strand.rs`) | Same invariant on the profile-ref path | ⚠️ Invert in Phase 5 |
| `runner_passes_prompt_via_stdin` / `runner_passes_strand_via_at_syntax` / `runner_passes_event_metadata` (adapters) | Current wire format: one merged user prompt on stdin | ✅ Green — defines the shape being split |
| `context_providers` tests (`tie-off block must lead the prompt`, consumer gating, pending-events ordering) | Single-string assembly in `AgentEventsContextProvider::build_full_context` | ✅ Green — the "leads the prompt" invariant becomes "leads the **system** prompt" |
| `process_strand` event-enforcement tests | Contract visibility drives re-ask behaviour | ✅ Green — must stay green with the contract in the system prompt |

## Test Gaps

- No test asserts the `--append-system-prompt` flag is present and carries
  the tie-off core / event contract / profile in that order.
- No test asserts stdin **excludes** the instruction blocks and includes
  only task + trigger + pending events.
- No test for the frozen artifact (written, byte-matches the rendered
  prompt, record emitted, session-id mapping).
- No test for resume re-supply and the drift warning.
- No compaction-survival acceptance probe (mock CLI that echoes its argv
  to stdout, asserting the contract is in the system prompt and absent
  from the transcript — the pirate-probe pattern).

## Phases

### Phase 0: Split the payload at the domain/application boundary

`ExecutionContext` gains a `system_prompt` payload; `prompt` shrinks to
runtime data. `AgentEventsContextProvider::build_full_context` is split
into the **instruction part** (tie-off block + event block) and the
**runtime part** (pending-events section); `process_strand` routes them
to their respective fields. Adapters temporarily keep behaviour by
concatenating — no wire change yet. Pane: unit tests naming the two
payloads (split correctness, ordering, gating preserved).

### Phase 1: pi-stdio emits `--append-system-prompt`

The adapter renders the system payload into the spawn args and strips it
from stdin. **Decision (verify first):** pi's flag accepts "text or file
contents" — prefer passing the frozen-artifact **path** (no argv bloat,
nothing readable via `ps`); fall back to inline text if path semantics
don't hold. One flag per section (tie-off, event, profile) for argv
inspectability if inlining. Pane: mock-CLI (`KNOT_TEST_CLI_PATH`)
assertions on argv and stdin.

### Phase 2: pi-json and pi-rpc adapters

Both share `PiJsonAgentRunner::build_prompt_with_context` (pi_rpc
delegates); apply the same split. For RPC the flag is still a spawn-time
CLI arg — the `prompt` command carries only the runtime payload.
Pane: mock tests for both adapters mirroring Phase 1's assertions.

### Phase 3: The frozen prompt artifact + deterministic session mapping

Knot mints the pi session id at spawn (`pi --session-id <uuid>`) so
run ↔ artifact ↔ session is deterministic (today stdio surfaces no
session id). Render → write `tie-offs/<rig>/prompts/<session-id>.md`
before spawn → emit the `PROMPT-FROZEN` service-log record. Pane:
artifact content + record assertions over the mock harness.

### Phase 4: Resume re-supplies the frozen prompt

`session_resume.rs` (compaction restarts, inactivity restarts,
event-request injection) passes the frozen artifact via
`--append-system-prompt`; renders the current rig config and diffs —
`[KNOT][EVENT] type=PROMPT-DRIFT` when they differ. **Decision:**
resume runs under the **frozen** prompt (the session started under it;
mid-session instruction changes are the drift we want to surface, not
apply); fresh render only as fallback when the artifact is missing.
Pane: resume-path mock tests incl. the drift warning firing.

### Phase 5: Retire the old invariants + changelog

Invert the two `no_system_prompt_flag` tests into positive assertions.
Add the `knot-update` changelog entry (breaking change: prompt placement
— third-party tooling that parsed the first user message for the tie-off
block must read the system prompt / artifact instead). Full suite +
clippy green. Pane: the whole already-green suite.

## Notes

- **Ordering is contractual**: tie-off → event → profile (the profile
  reads as the persona speaking under the standing contract).
- The `# Tie-Off` block "leads the prompt" invariant survives as
  "leads the system prompt".
- Pending-events and the trigger line stay in user space — they are
  per-invocation runtime data, not instruction.
- `--append-system-prompt` is repeatable; the hard-coded cores stay
  non-overridable — only the assembly location moves. Plan 102's
  two-document split is the input shape this plan consumes.
- **Skill/doc ripple (COMPLETED post-landing):** `knot-template`,
  `knot-create`, `knot-dispatch`, `knot-init` + glossary, and the
  `knot-update` changelog were updated for the system-prompt placement
  and deployed to `~/.agents`. The pirate-probe test skill was
  intentionally not created.
- Open question: artifact retention/rotation alongside `tie-offs.log`
  rotation — same policy, separate act; not in scope.
- Risk: providers with small argv limits — mitigated by the
  pass-the-path decision in Phase 1.
