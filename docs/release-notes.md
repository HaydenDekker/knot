# Release Notes

## v0.50.0 — 2026-09-21

### Changed — Static Engine Token for Rig-Scoped System Events (Plan 092)

Rig-scoped system events (currently `QueueIdle`) now subscribe by a
**static engine token** instead of the rig directory's basename. The
rig-name token encoded a deployment detail — the name the rig directory
happened to be called — so a rig built from a template under another
name (or a renamed rig directory) silently lost every rig-scoped
subscription: the event was logged, no event file was dispatched, no
consumer fired, and nothing was logged about the gap. One process per
rig makes the token redundant — the process boundary and the runtime
root (`tie-offs/<rig>/…`) already scope every dispatch to the running
rig.

- **Canonical form: `event:knot:<EventId>`** (e.g.
  `event:knot:QueueIdle`). The token is the Knot engine — invariant
  under rig-directory renames and template reuse. Wildcard
  (`event:*:QueueIdle`) is unchanged.
- **The rig-name form is deprecated, not broken.**
  `event:<rig-id>:<EventId>` keeps working in this release; removal is
  reserved for a later breaking release. Existing rigs need no
  migration.
- **Event-file provenance unchanged.** The `target-knot:` frontmatter
  of a dispatched system event still carries the *actual* rig id
  regardless of the subscription form — only matching changed.
- **Zero-consumer dispatch is no longer silent.** A rig-scoped emit
  that matches no consumer logs one service-log line:
  `[KNOT][SYSTEM] event=<EventId> rig=<rig> — 0 consumers matched`,
  extended with `near-miss subscription(s): <knot>
  (event:<token>:<EventId>)` when a knot subscribes to the same event
  id with a non-matching producer token — the rename-mismatch
  signature, named directly. Scoped to rig-scoped emits: zero
  consumers is the normal state for knot- and loom-scoped events.
- No rig document format changes; no project document migration
  required (the new form is recommended for new subscriptions).

## v0.49.0 — 2026-09-20

### Fixed — Per-Event Enforcement (Plan 091)

Event enforcement now checks **per-event completeness** instead of
merely "does the tie-off contain at least one event block". The 0.48.x
shape left a silent gap: a knot whose subscribers expected several
`event:` sources could acknowledge some events — even only with
`occurred: false` blocks — and the enforcement gate saw *an event block
present* and passed it through. The remaining events simply were never
delivered, and the consumer knots that subscribe to them never ran. A
live rig (a `prd-config-check` knot narrating that it emitted
`PlanRequested` without emitting the block) exposed exactly this: the
tie-off carried three `occurred: false` acknowledgements, the
enforcement gate passed it, and the subscriber waiting on
`PlanRequested` sat silently.

- **The missing set is computed per event.** The expected set comes
  from the same subscriber context injected into the prompt
  (`event:` sources of loom-level and knot-level subscribers, plus the
  `TasksIncomplete` self-continuation entry). Every expected event ID
  with no structured block in the tie-off — an `occurred: true` or
  `occurred: false` block both count as present — is *missing*.
  Narrative mentions are not acknowledgements; the structured block is.
- **Zero blocks is the all-missing case.** A tie-off with no event
  blocks at all now logs `KnotEventsMissing` with the full expected set
  as the missing set — the old behaviour, preserved and generalised.
- **`event: None` is a blanket acknowledgement.** As before, a
  `None` block satisfies the whole list ("nothing happened"); it
  never appears in a missing set.
- **`TasksIncomplete` is never enforced.** It is a conditional
  self-continuation signal, not a subscriber acknowledgement; when it
  is the only expected entry, enforcement does not run at all.
- **The re-ask names the missing events.** The follow-up prompt now
  asks the agent to emit blocks for *exactly* the missing event IDs
  instead of the zero-block wording. Follow-up events are dispatched
  normally (subject to `occurred` filtering).
- **`KnotEventsMissing` carries both sets.** The variant gains a
  `missing_events` field alongside `expected_events` (the old field
  now means the full expected set, not "what was missing"); the
  service log renders `expected=…` and `missing=…`.
- **One follow-up, then the gap stands.** As before, one re-entry is
  attempted; a follow-up that still leaves events unacknowledged
  produces a second `KnotEventsMissing` naming the still-missing set.
  Processing completes regardless — missing events are a recorded
  outcome, not a failure.
- No rig document format changes; no project document migration
  required.

## v0.48.0 — 2026-09-12

### Improved — Concise In-Session Retry Prompts (Plan 090)

A session-resume retry re-entering an existing pi session (`--session-id`) no
longer re-sends the full original prompt (profile persona + knot/strand
prompt + trigger line) plus the note. The re-entered session already holds
all of that; each of the up to 10 retries was re-sending it — pure token
overhead on every retry, and worst on exactly the attempts fired because
context ran out (post-compaction restart, water-mark handoff,
compaction-interrupt recovery), where the re-sent prompt could burn the
budget the compaction just saved and trigger a second compaction of a
context that was just shrunk.

- **In-session retries are the note only.** The retry prompt is exactly the
  cause-specific note — the final-response request (default), the
  inactivity restart note, the compaction restart note, or the water-mark
  handoff note — with an empty profile prompt (the `inject_event_request`
  precedent). Notes no longer accumulate across attempts: each retry is
  exactly its own note.
- **Fresh restarts are unchanged.** The one retry that runs without a
  session ID (an inactivity stall before the session ID was captured)
  keeps the full composed prompt plus the note — a fresh process has no
  history, and the full prompt is the only copy of the task instructions.
- **The `@strand-file` attachment stays on every attempt**, so the agent
  can still re-read the original task file if a compaction summarised the
  original instructions away.
- Bounds, budget math, events, and terminal-error classification are
  unchanged; no project document format changes (no rig-document
  migration required).

## v0.47.0 — 2026-09-12

### Feature — A Compaction No Longer Ends the Attempt (Plan 089, phases 8–13)

v0.46.0 recovered the *overflow* shape where pi's compaction died mid-turn.
The 2026-09-11 rig run showed the same family of failures coming from
**proactive** (`threshold`) compactions, and from three places Knot itself was
cutting them short. A compaction on that rig cost a lost turn, a re-prompt and
a second 30–47 s summarisation — or the whole attempt.

**What changed — the run waits for the compaction, then asks for its answer:**

1. **Settle-based teardown (pi-rpc)** — `agent_end` means the model's turn is
   over, not the prompt: pi still runs its post-agent compaction inside the
   same awaited prompt and emits `agent_settled` afterwards. Knot now closes
   the child's stdin on the **settle**, falling back to `agent_end` only after
   a short settle window with no compaction open. `pi-rpc` exits at stdin EOF,
   so the previous `agent_end`-immediately rule was itself killing pi
   mid-summarisation (five `threshold` interruptions on the rig — Knot's own
   doing, not pi's).
2. **In-session continuation** — when a settled turn carries no text because a
   compaction just completed, Knot sends **one** follow-up `prompt` on the live
   channel ("reply with your final answer now — do not start any new task")
   and takes the answer from the same process, logged as **`TurnContinued`**.
   The in-session sibling of `SessionRestarted`: no new process, no re-sent
   strand, no second summarisation. Unanswered, the ordinary empty-response /
   session-resume path still applies, bounded by the existing liveness rules
   (no new timeout).
3. **Interruptions for any reason** — an unclosed compaction span
   (`compaction_start` with no `compaction_end`) is now an interruption
   whatever pi's reason was, so a `threshold` death gets the v0.46.0 manual
   compact + session restart. With (1) in place Knot no longer stops the
   process it watches, which is what made the shape ambiguous before.
4. **A compaction span counts as activity** — pi writes nothing while it
   summarises, so the span boundaries now stamp the inactivity watchdog and
   the inactivity test stands down while a span is open. The **total budget is
   never suspended**: a wedged compaction is still killed on the deadline and
   reported as a `Timeout`, which is what it was. Both stream runners share one
   span-tracking helper.
5. **Observability** — `CompactionStarted` on a resumed attempt carries the
   session id from the first line (Knot seeds it from the `--session-id` it
   passed; the RPC stream has no `session` header and `get_state` can answer
   late, which is why the rig's rows printed an empty `session=`); an empty
   response names the compaction it arrived in; and startup logs **
   `RunAbandoned`** once per queue entry restored from the previous service — a
   run that stopped mid-strand, re-run from scratch rather than resumed.

**Operator-visible changes:** threshold compactions no longer end an attempt;
`TurnContinued` and `RunAbandoned` appear in the service log and loom logs;
`CompactionInterrupted` can now name `reason: threshold`; `AgentInactivity`
cannot fire inside a compaction span. No document-format change — no migration
(`knot-update`: nothing to do when upgrading).

## v0.46.0 — 2026-09-11

### Feature — Interrupted Overflow Compaction: Manual-Compact + Session-Restart Recovery (Plan 089)

Plan 088 (v0.45.0) made compaction always-on and observable, but one overflow
shape was still classified as **terminal** even though it is *resumable*: the
in-process overflow compaction **started but never reported completion**
(`compaction_start { reason: overflow }` with no `compaction_end`, and no
terminal `errorMessage`). That is pi's overflow recovery dying *mid-turn* — a
process kill, an inactivity stall, or a stream gap — leaving the context still
over-full. Previously this fell into `ContextLimitReached` (terminal,
"context limit reached") and the strand failed even though the session was
still live and the context could be shrunk.

**What changed — the shape is reclassified to a resumable `CompactionInterrupted`
and Knot recovers it with an out-of-band manual compact:**

1. **Classification (pi-json / pi-rpc)** — the overflow classifier now
   distinguishes the *interrupted* shape (a `compaction_start` was observed
   and no `compaction_end` completed the span, with no terminal
   `errorMessage`) from the terminal shapes (recovered-then-failed, or
   attempted-but-could-not-fit). It yields a new **resumable**
   `PortError::CompactionInterrupted` that carries the live session id and pi's
   reason — instead of the terminal `ContextLimitReached`. A
   `compaction_end` with no matching `compaction_start` is a stray (not an
   interruption).
2. **Manual compact (pi-rpc)** — a new `AgentRunner::manual_compact(ctx,
   session_id, custom_instructions)` re-opens the *same* session with
   `--session <id>` and sends a `compact` RPC (with a fixed operator note as
   `customInstructions`), bounded by the remaining budget. It returns a
   `CompactionRecord` on success and `PortError::ManualCompactionFailed` on
   timeout / error / abort. A manual compact is a first-class pi command that
   **always** emits a `compaction_end` (success or failure), so the interrupted
   span is closed and the context is actually reduced — the mid-turn
   `agent_end`-gated path that originally failed is bypassed entirely.
3. **Recovery (session-resume usecase)** — on a `CompactionInterrupted`, the
   usecase runs **one** manual compact (bounded per failed execution), records
   the boundary events, and — on success — re-enters the session with a restart
   note ("your context was just compacted — continue from the compacted
   state"). If the explicit compact itself cannot reduce the context the run is
   terminal. A second interruption (if the re-entry overflows again) is left to
   the normal retry machinery.
4. **New events (loom log + service log)** — `CompactionInterrupted`,
   `ManualCompactionSucceeded` (with `tokens_before`), `ManualCompactionFailed`
   (with the error), and `SessionRestarted`. The good shape in the service log:
   `CompactionInterrupted` → `ManualCompactionSucceeded` → `SessionRestarted`
   → the strand continues.
5. **Driver hold (pi-rpc)** — an in-flight `compaction_start` now **holds** the
   teardown past `agent_end`: the driver waits for the matching
   `compaction_end` (or a stray `compaction_end` is ignored) instead of tearing
   down mid-compaction, which is what cut the span short in the first place.

**Operator note:** the manual-compact `customInstructions` is a fixed operator
note (default) — it tells the compact to preserve the durable task state, open
work items, and checklist/state pointers so the next session continues without
re-deriving context. No new per-strand knob; the `compaction.enabled` escape
hatch is unchanged. See `docs/concepts.md` (Context Compaction) and
`docs/troubleshooting.md` (`CompactionInterrupted`).

## v0.45.1 — 2026-09-11

### Fix — Overflow Classification: "Compaction Attempted but Could Not Fit" vs. "Compaction Did Not Run"

Plan 088 (v0.45.0) added live compaction observation and always-on
compaction, but the overflow **error message** kept a misleading branch. When
an over-full context surfaced as a provider overflow error with an empty
final response, the message was chosen by `terminal_overflow(...)`, which
only matches the *recovered-then-failed* shape (a failing overflow record
preceded by a successful one). Any other overflow failure fell through to a
hardcoded "pi auto-compaction **did not run** … enable compaction with
`.pi/settings.json`" message — even when compaction had actually run and
simply could not shrink the context.

**Observed incident:** a plan-author run whose single model output (~26.7k
tokens) pushed the context from ~123k straight past the 150k window in one
turn. pi's threshold compaction only runs *between* turns (at `agent_end`),
so it could not prevent the mid-turn overflow; the overflow recovery **did**
start (`CompactionStarted reason=overflow`) but its summarisation could not
reduce a near-window context below the limit. Knot's message wrongly told the
operator to "enable compaction," which was already on.

**Fix — a single classifier
(`PiJsonAgentRunner::classify_overflow_failure`) replaces the old
`terminal_overflow(...).or(...)` fallthrough at both runner call sites
(pi-json and pi-rpc), distinguishing three shapes:**

1. **Recovered, then failed** (terminal) — unchanged: a failing overflow
   record preceded by a successful one. Message is pi's recovery-failure
   `errorMessage` (or a generic "cannot fit after compaction").
2. **Compaction attempted but could not fit** — a provider overflow error
   with an empty response **and** a compaction was attempted (a live
   `compaction_start` was observed, or a `compaction_end` record exists).
   Message: "pi **attempted** compaction (reason(s): …) but the context
   still cannot fit … Compaction is **already enabled**; lower the model's
   `maxTokens` … or use a larger-context model."
3. **Compaction never ran** — a provider overflow error with an empty
   response and **no** compaction event (compaction disabled in pi
   settings). Message unchanged: "pi auto-compaction did not run … enable
   compaction with `.pi/settings.json`."

The distinction between 2 and 3 is observable because the live
`compaction_start` (and any `compaction_end`) are recorded in
`compaction_starts` / `compactions` even when the recovery produces no
successful compaction.

**Root-cause note for operators (the rig-side fix):** pi's threshold
compaction runs *between* turns, at a context of `contextWindow −
reserveTokens` (default `150000 − 16384 = 133616`). A single turn's output
(up to the model's `maxTokens`) added to a just-below-threshold context can
exceed the window — that is the incident. The invariant that prevents it is
**`reserveTokens ≥ maxTokens`**: then a max-size output from a
just-below-threshold context still lands under the window. The rig's model
had `maxTokens: 32768 > reserveTokens: 16384`, so the invariant was violated.
Fix: raise `compaction.reserveTokens` to ≥ the model's `maxTokens` (e.g.
`32768`), or lower the model's `maxTokens` to ≤ `16384`, or use a
larger-context model.

**No document or format change** — profiles, knots and looms are unchanged;
no migration required.

## v0.45.0 — 2026-09-11

### Extended — Compaction Assurance: Auto-Compaction Always On, Overflow Recovery Continues the Session (Plan 088)

Knot's context-overflow recovery (plan 079) relied on pi's built-in
compact-and-continue, which pi settings alone control. Two gaps made that
reliability invisible and fragile: compaction was off by default in the
global pi settings (so legacy rigs ran overflow-unrecoverable, with only a
startup warning), and a compaction's outcome was only visible after the
fact — and only when it *succeeded*.
v0.45.0 closes both: Knot now guarantees compaction is on for rig sessions,
observes each compaction **live** as a span, and carries the session id
across every re-entry.

- **Startup self-heal (always-on compaction).** At startup, before watcher
  registration, Knot checks the effective pi compaction setting (project
  `.pi/settings.json` over global `~/.pi/agent/settings.json`; pi's default
  is enabled). When it resolves to disabled (the global file explicitly
  says `false` with no project override — the shape of rigs initialised
  before `knot-init` seeded the project file), Knot **merges**
  `{"compaction": {"enabled": true}}` into the project file, preserving
  every existing key. The project file is Knot's own (knot-init seeds it;
  the service maintains it); the global file is the operator's and is never
  written. An **explicit** project-level `compaction.enabled` (true or
  false) is honoured — an explicit `false` is an operator opt-out and still
  raises the (now reason-annotated) plan 080 startup warning, as do an
  unparseable project file (never clobbered) and a write failure.
- **Live compaction spans.** pi emits `compaction_start` / `compaction_end`
  events in the agent's JSON stream. Both runners (pi-json, pi-rpc) now
  observe these **live** (on the stream reader / driver line loop) and the
  session use case records the span boundaries as new loom events, each
  emitting a system event like the existing `ContextCompacted` pattern:
  - `CompactionStarted` — the span began: `session_id`, `reason` (pi's
    `"threshold"` / `"overflow"` / `"manual"`), `attempt`.
  - `ContextCompacted` — the span ended **successfully**. Operator-facing
    context-pressure signal; **shape unchanged** (only the timing moved
    from after-the-invocation to live).
  - `ContextCompactionFailed` — the span ended **without success** (pi's
    `errorMessage`, or an aborted span): `error`, `aborted`, plus the same
    fields. Plan 079's `error.is_none()` filter — which silently dropped
    failed compactions — is now a routing decision.
  - `ContextCompacted` **fires only on success** (the success signal);
    the failed/aborted ends route to `ContextCompactionFailed`.
- **Continuity across re-entries.** The session id is carried from the
  runner: when a final-response nudge (plan 078) or a session-resume retry
  re-enters a session, it re-enters the **runner-captured** session id from
  the invocation's stream — so a compaction observed mid-run never loses
  the session identity for the retry.
- **RPC parity for overflow recovery.** The `pi-rpc` adapter now records
  `compaction_end` events (reason, `tokensBefore`, `errorMessage`,
  `willRetry`, `aborted`) from its stream exactly as `pi-json` does —
  including the terminal-overflow fail-fast (`ContextLimitReached`) —
  with mock-CLI tests covering the recovered (success) and terminal
  (fail-fast) streams.
- **`compaction_starts` metadata.** Each runner's invocation metadata now
  carries the list of `compaction_start` reasons observed in the stream
  (the span begins are visible in the invocation record, not just the
  ends).

**No migration required.** The loom-log and service-log formats gain two
new line shapes (`CompactionStarted`, `ContextCompactionFailed`); existing
`ContextCompacted` lines are unchanged. No rig document or front-matter
format changed.

**Service-log shapes added:**

```
[KNOT][EVENT] CompactionStarted loom=<loom> knot=<knot> strand=<path> session=<id> reason=<reason> attempt=<n>
[KNOT][EVENT] ContextCompactionFailed loom=<loom> knot=<knot> strand=<path> session=<id> reason=<reason> error=<message> aborted=<bool> attempt=<n>
```

## v0.44.1 — 2026-09-10

### Fix — Continuation Background Accumulation Round-Trips Block Scalars (Plan 087, Phase 4)

The self-continuation chain's "accumulated, hop-labelled"
`background-additional` (v0.44.0 / plan 086) did **not** accumulate across
hops. Each continuation front-matter block-scalar collapsed to the literal
`|` marker, so the next hop's continuation carried an empty `[hop N]` label
and a bare-`|` `## Next Task Context` — the prior hops' background and the
agent's newly-appended facts were both lost.

**Root cause:** the two front-matter readers are naive `key: value`
line-splitters with no YAML block-scalar (`|` / `>`) support.
`tieoff_parser::parse_frontmatter` (which parses the agent's ```markdown
tie-off into the event payload) and
`strand_event_metadata::parse_yaml_frontmatter` (which reads the prior
continuation file's front-matter to recover the incoming accumulation) both
recorded a `key: |` block scalar as the value `"|"` and dropped the indented
body. That broke **both** halves of the invariant — pass the incoming
accumulation **through** and **append** the agent's new input.

**Fix (read-side only; the writer already emitted a correct block scalar):**

- `tieoff_parser::parse_frontmatter` is now block-scalar-aware (and public),
  operating on raw, indentation-preserving lines: on `key: |` / `key: >`
  (optional `+`/`-`) it collects the following indented body, de-indents by
  the common leading whitespace, and joins with newlines.
- `parse_event_block` passes raw front-matter lines (previously it pre-trimmed
  every line, destroying the body's indentation).
- `parse_yaml_frontmatter` delegates to the shared parser, so a continuation
  file's `background-additional: |` round-trips.

**No document or format change** — the on-disk continuation front-matter is
unchanged. No migration required. One behaviour note: a top-level key with a
leading space is no longer parsed as a key (top-level keys are column-0;
indented lines are block-scalar bodies); Knot-written files are always
column-0, so this never bites in practice.

## v0.44.0 — 2026-09-10

### Extended — Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget (Plan 087)

Plan 086's self-continuation — a `task-loop` knot resuming a bounded task
chain after its context water-mark, delivered into its own existing input —
was scoped to filesystem-strand knots. v1 left event-source knots
(`strand-dir: event:<producer>:<EventId>`) out: their continuation was never
written, so the batch paused with work remaining (re-entry fell back to the
explicit event chain or the overflow safety net). v0.44.0 completes the chain
for event-source knots and refines 086's budget model so a lengthy event
queued between a handoff and the continuation's dequeue no longer erodes the
continuation's budget.

- **Event-source knots now self-continue.** A continuation is delivered into
  the knot's **existing input** — its event dispatch dir
  (`tie-offs/<rig>/<loom>/<event-id>/`) — the same dir the knot's existing
  watcher already watches, bound to `(loom, knot)`. The watcher fires
  `StrandEvent::Created` for that knot, so the batch re-enters through the
  normal dispatch pipeline. No new subscription, no new queue, no second
  `strand-source`. Filesystem-strand behaviour is unchanged (the continuation
  still lands in the knot's own strand dir).
- **Queue-wait-exempt batch execution budget.** 0.43.0 stamped an absolute
  `batch-deadline-epoch` and, at dequeue, subtracted elapsed wall-clock
  (`remaining = deadline − now`) — so a lengthy event queued *between* the
  handoff and the continuation's dequeue stole budget the knot never used.
  The continuation front-matter now carries the **remaining execution budget
  in seconds** (`budget-secs`) plus the batch's absolute origin
  (`batch-start-epoch`). Only *execution* decrements the budget
  (`budget-secs = incoming − execution_secs`, where `execution_secs` is the
  hop's wall-clock dequeue→handoff span); **queue wait never does**. The
  whole batch — all hops, including the first — shares one total execution
  budget of `profile_timeout`, and the budget is never reset at a handoff
  ("fresh context, never a fresh budget").
- **Honest observability.** The `TasksIncomplete` loom event and the
  `[task-loop] handoff` service-log line now record the **stamped remaining
  budget** (`remaining=<B>s`) and the delivery path
  (`source=filesystem|event`), and are emitted **only when the hop actually
  happens**: on the max-continuations cap or an exhausted budget the
  continuation is suppressed and a `BatchIncomplete` is recorded instead, so
  the log never claims a hop that did not happen.
- **Durable + replayable.** The continuation is written to disk before the
  per-turn project commit, so it is captured in the project's git commit (the
  rig repo stays source-only) — replay the batch hop by hop by navigating the
  project's commits.

**Compatibility:** a continuation file written by a 0.43.0 (086) build
carries only `batch-deadline-epoch`; the 0.44.0 reader derives its budget
from the deadline at read time (a one-time read-only shim). New
continuations never write `batch-deadline-epoch`; the shim is removed after
one release.

## v0.43.0 — 2026-09-08

### New Capability — Graceful Task Handoff: Checkpointed Continuation Chains (Plan 086)

A `task-loop` knot that outgrows a single context window used to either
overflow (losing uncommitted work) or re-derive its scope from scratch on
every re-dispatch. Plan 086 lets such a knot work through a **durable,
rig-owned checklist** across a chain of bounded sessions: it **hands off
gracefully** when its context water-mark is crossed and a **fresh session
resumes from the checklist** — so a long batch completes across as many
hops as it needs, each hop starting from the on-disk state.

- **`TasksIncomplete` self-continuation** — a new loom event a knot
  acknowledges in its own tie-off. When `ctx-wrap-up-limit` is set on the
  alias and the agent is steered at the water-mark, it commits in-flight
  work, updates the checklist, and emits a fenced `markdown` block with
  `event: TasksIncomplete`. `occurred: true` (with `next-task-context` —
  the operational resume brief — and `background-additional` — thin
  persistent facts) dispatches a **continuation**: the same knot re-run in
  a fresh session that resumes from the checklist. `occurred: false` is
  the explicit “batch complete” declaration that ends the chain.
- **The chain obeys the original clock.** A continuation never resets the
  timer: its `profile_timeout` is derived from the `batch-deadline-epoch`
  stamped on its event, so the whole chain shares the first session’s
  budget (“fresh context, never a fresh budget”). The `MAX_CONTINUATIONS`
  cap (10) and the deadline bound the chain; hitting either records a
  terminal `BatchIncomplete` loom event (`reason: "caps"` or
  `reason: "deadline"`) and drains the queue.
- **Knot stays task-blind.** It neither owns nor parses the checklist — its
  format, location, and authorship are the rig designer’s. The
  agent-emitted `TasksIncomplete` is the only task-specific seam, and its
  body is a **pointer block** to durable state (the checklist + committed
  files), never a re-statement of context.
- **The contract is delivered at the water-mark, not the base prompt.** The
  `TasksIncomplete` description is no longer injected into every base
  prompt for a water-marked alias — that `Subscriber Events` block was
  ~900 tokens of the base context and shrank the working headroom that
  decides how much real work fits before the steer. It now rides in the
  self-contained wrap-up steer, which also specifies the `event:`
  front-matter field the tie-off parser requires (the old base prompt
  carried it via the generic “Event Format” example). Regular subscriber
  events still inject at the start.
- **Two adapters, one seam.** The water-mark steer — and hence the handoff —
  is `pi-rpc` only; on `pi-json`/`claude` the existing overflow/timeout
  terminal handling re-dispatches the idempotent knot, which resumes from
  the checklist.
- **Fixes from rig testing.** (a) the `pi-rpc` adapter no longer passes the
  strand as a `@{path}` CLI arg (pi RPC mode rejects it) — the strand
  content is injected through the stdin prompt instead; (b) the tie-off
  parser recovers a fenced `markdown` block left open at EOF, so a
  correctly front-matter-delimited handoff is not dropped when a model
  omits the closing fence.

**Tests** — unit tests cover continuation dispatch (deadline-derived
timeout, cap, `occurred` filtering), the self-contained steer contract
(`HANDOFF_NOTE` carries the `event:` field), and the unclosed-fence
recovery. Verified empirically on a `pi-rpc` rig: a 6-item checklist batch
completes across a water-mark-driven continuation chain, ending in an
`occurred: false` “batch complete” declaration. The full suite passes.

## v0.42.0 — 2026-09-07

### New Capability — `pi-rpc` Runner + Context Wrap-Up Steering (Plan 084)

A long agent run that exhausts its context window today dies mid-task,
leaving an uncommitted working tree and a session that cannot be
resumed. This release adds an opt-in `pi-rpc` agent runner that watches
the live context usage and **steers** the session to wrap up before it
runs out, so context exhaustion ends in a clean handoff.

- **`pi-rpc` adapter** — a new `adapter: pi-rpc` value (the
  `pi-json` runner is unchanged and remains the default). The adapter
  launches `pi --mode rpc` (JSONL command protocol over stdin/stdout),
  sends the initial `prompt`, and reads pi's stdout line-by-line. On
  every `turn_end` it samples `get_session_stats` — always capturing
  usage for observability (parity with `pi-json`) and, when a
  `ctx-wrap-up-limit` is set, using the sample as the steer decision
  point.
- **Context wrap-up steering** — a new per-model-alias config key
  `ctx-wrap-up-limit` (tokens) in `rig/models.yml`. When the sampled
  context tokens cross the limit, the adapter sends **one** `steer`
  (the wrap-up prompt) at the next turn boundary — telling the agent to
  stop starting new work, commit all complete work, update its
  progress, note what is incomplete and where it left off, and produce
  its final tie-off. The steer fires once per run, regardless of
  whether pi's own auto-compaction is enabled (the steer is the remedy
  either way). A single-run warning is emitted (stderr) when the limit
  sits within pi's default reserve of the context window, where pi may
  compact before the limit is reached.
- **`ContextWrapUpSteered` event** — a one-shot loom event
  (loom-log) recording the steer: `loom_id`, `knot_id`, `strand_path`,
  `session_id`, `context_tokens`, `limit`, `attempt`. Rendered in the
  service log, serde round-tripped, and logged from the session-resume
  Ok path via the new `WrapUpRecord` in `AgentInvocationMetadata`.
- **Config resolution** — `ctx-wrap-up-limit` resolves through
  `ModelRef`/`AgentConfig`; a value of `0` filters to `None`
  (disables steering), so the default is off. Direct model specs
  (no alias) have no `ctx-wrap-up-limit`.
- **Tests** — the unit tests drive the real adapter end-to-end against
  a mock `pi` binary (`with_cli_path`), covering success (response,
  session-id, and usage captured from the `agent_end` frame),
  steer-fires-once-when-crossed, no-steer-below-limit, and
  steer-despite-disabled-compaction. All 1332 tests pass.

**Deferred next change:** `pi-json` removal (the plan's final
migration, done after `pi-rpc` has proven itself in rig service).

## v0.41.1 — 2026-09-07

### Quality of Life — Event-Parse Log Flags + Root-Anchored Rig `.gitignore` (Plan 085)

Two small operational defects surfaced during rig runs. Both are fixed
in this patch — no rig-document changes, no behavioural change to
dispatch or processing.

- **Event-parse line shows the `occurred` flag.** When Knot extracts
  structured agent events from a completed tie-off, the console
  diagnostic now renders each event with its status —
  `event parse (knot=author): 2 event(s) found — PlanCreated=true,
  SpecReviewed=false` — instead of a bare id list that hid which events
  actually fired (`true`, dispatched to consumers) versus
  acknowledgements (`false`, counted for enforcement, never
  dispatched). Stderr diagnostics only; the dispatch filter on
  `occurred` is unchanged.
- **Rig exclusion is root-anchored.** The `.gitignore` entry Knot
  appends to the parent repo is now `/{basename}/` (e.g. `/rig/`)
  instead of the unanchored `rig/`. Git matches a slash-less pattern at
  **any depth**, so the old entry silently also ignored `tie-offs/rig/`
  — the project-versioned runtime data (tie-offs, service log, event
  queue, `state.json`). The anchored entry matches only the top-level
  rig directory.
- **Force-migration of existing entries.** On the first 0.41.1+ start,
  `ensure_rig_repo()` rewrites any pre-existing bare `{basename}/` line
  to `/{basename}/` in place — the marker and every other line
  (including user-authored entries) are preserved verbatim. An already
  anchored line short-circuits to a no-op, so the migration is
  one-time and idempotent. The tracked-by-parent guard is unchanged
  (untracking remains a user decision); the logged untrack command now
  reads `git rm -r --cached /rig/`.
- **Tests** — the three existing `ensure_rig_repo` entry tests now
  assert the anchored form, and two new tests pin the migration: a
  pre-0.41.1 file (marker + bare `rig/` + user line) converges to the
  anchored form with no duplicate and a no-op second run, and an
  already-anchored file is left byte-identical.

## v0.41.0 — 2026-09-07

### Feature — Consolidated Service Log + Change-Driven State Writes (Plan 083)

Observability ran on three moving parts: the per-loom `.loom-log` JSONL
file, the rig-level `.rig-log` JSONL file, and `state.json` — rewritten
on a 5-second tick whether or not anything had changed (so its mtime
churned and `updated_at` lied about when the state last moved). Plan
083 collapses the two log files into the one log the service already
had — its **stderr** — and stops the no-op `state.json` writes.

- **One line per record on stderr** — every domain event (loom
  lifecycle, strand processing, timeouts, `SessionResumed`,
  `QueueIdle`, …) renders as a single-line
  `[2026-09-07T15:35:57+10:00] [KNOT][EVENT] KnotCompleted loom=… knot=…
  strand=… tie-off=…` record: one emit-time timestamp, the event's
  fields as `key=value` pairs (only the fields that variant carries —
  no `timestamp=` field, no `"null"`), one physical line (no embedded
  newlines). Every `state.json` write renders the same way:
  `[KNOT][STATE] initial snapshot looms=N knots=N profiles=N queue=N` at
  startup, then `change` deltas naming exactly what moved (loom/knot/
  profile add/remove, knot/profile field changes, queue add/drain).
- **In-memory run activity** — `.loom-log` and `.rig-log` are gone:
  run events live in a per-process `RunActivity` (the in-memory adapters
  own the `[EVENT]` emit). The existing query use cases (`GetLoomActivity`,
  `KnotStatus`) keep working over the same log ports, now backed by the
  current run instead of a file. Nothing is cleared at startup anymore —
  the Plan 072 truncation mechanism is removed, and legacy log files a
  0.31.0 migration may have moved are **inert** (never read, written,
  or appended to; safe to delete).
- **Change-driven `state.json` writes** — the state writer diffs the
  freshly derived state against the last *written* state (ignoring
  `updated_at`, which is the only field that ever differs between
  identical snapshots). Identical content and an existing file → no
  write, no mtime churn, no log line. A real change or a missing file
  (fresh rig) → write, plus the `[KNOT][STATE]` delta line.
  `state.json` keeps its schema; `updated_at` now means *last actual
  change*, and between changes the file stays byte-identical (operators
  can watch its mtime and know the rig is idle).
- **Durable records unchanged** — the tie-off files remain the audit
  record of completed work, and `knot-service.log` (appended by the
  `knot-start` skill, which appends the service stderr) remains the
  durable *operational* record across restarts. Starting Knot
  foreground with plain `cargo run` still works — you just watch
  stderr.
- **Tests** — every retired-log assertion was converted (no vacuous
  passes): binary-level `tests/consolidated_log.rs` pins the `[EVENT]`
  run sequence, the absence of `.rig-log`/`.loom-log`, the baseline +
  change-driven `state.json` behaviour (idle ticks cause no write),
  burst-2-strand `queue+`/`queue-` deltas, and `knot step` baseline +
  delta lines; in-process suites assert on the durable surfaces
  (`state.json`, tie-off sections, event-queue files) instead of the
  retired files; the legacy-layout migration test now proves the
  migrated logs survive byte-identical and inert.

### Feature — System Event Subscriptions: Every Log Event Is Dispatchable (Plan 082)

A knot could only react to the events its *peer agents* chose to emit —
the system events Knot itself records (a run failed, timed out, went
idle, a loom stopped) were invisible to other knots. Plan 082 makes
**every system event Knot writes to the loom-log / rig-log also
dispatchable** to subscriber knots, through the existing `event:`
`strand-dir` URI — so you can react to knot outcomes, not just to
agent-emitted events.

- **Open, not curated.** Instead of a fixed 8-event allow-list, the
  vocabulary is the existing `LoomEvent` / `RigLogEvent` variant names —
  a new log-event variant is subscribable the day it lands. Run outcome
  (`KnotProcessing`, `KnotFailed`, `KnotCompleted`, `KnotEventsMissing`,
  `TimeoutExceeded`, `StrandIgnored`, `StrandSkipped`,
  `StrandProcessed`), retry/session (`SessionResumed`,
  `KnotEmptyResponse`, `AgentInactivity`, `ContextCompacted`), loom/knot
  lifecycle (`LoomStarted`, `LoomStopped`, `KnotRegistered`,
  `KnotDeregistered`, `KnotParseWarning`, `DirectoryCreated`), and rig
  lifecycle (`QueueIdle`). `EventsDispatched` is intentionally excluded.
- **Two new producer-token positions.** On top of the existing
  knot-level (`event:<knot>:<EventId>`) and loom-level
  (`event:<loom>:<EventId>`) subscriptions:
  - **Wildcard** `event:*:<EventId>` — match *any* knot in the rig (the
    "react to any knot that fails" monitor: `event:*:KnotFailed`).
  - **Rig-level** `event:<rig-id>:<EventId>` — a rig-scoped event
    (currently `QueueIdle`), using the rig's ID as the producer token.
- **Self-exclusion.** A system event is never dispatched back to the
  knot that produced it — a knot's own `KnotFailed` / `KnotCompleted`
  does not re-trigger itself. This is automatic and applies to system
  events only (a knot may still deliberately subscribe to its own
  agent-emitted events).
- **Per-attempt fan-out.** Retry events (`SessionResumed`,
  `KnotEmptyResponse`, `AgentInactivity`, `ContextCompacted`) fire once
  *per attempt*, not per run — a retried run fires a per-attempt
  consumer multiple times. Subscribers must be **idempotent** under
  re-delivery (standard knot discipline).
- **Emission is best-effort and non-fatal.** A failed dispatch never
  changes a run's outcome or blocks the pipeline (parity with
  agent-event dispatch); it surfaces on the console.
- **Additive — no migration.** Existing looms and agent-event
  subscriptions are untouched. New subscribers simply set `strand-dir`
  to `event:<producer>:<SystemEventId>` (producer = knot ID, loom ID,
  `*`, or the rig ID for `QueueIdle`).
- **Tests** — domain resolvers (pure), the emitter against a mock
  dispatcher + in-memory store (all scopes, producer tokens,
  wildcard/rig-level, self-exclusion, singleton-vs-batch seq), and
  mock-CLI harness end-to-end acceptance (`tests/system_event_subscriptions.rs`):
  failure → wildcard `KnotFailed` consumer runs, success →
  specific-producer `KnotCompleted` consumer runs, self-exclusion, and
  timeout → `TimeoutExceeded` consumer + rig-log entry (077/081
  no-failed-tie-off contract asserted alongside). The 056/058/059/070
  agent-event suites are untouched and green.

## v0.40.0 — 2026-09-03

### Feature — Inactivity Timeout: Kill Blocked Sessions, Restart with a Blocking-Call Note (Plan 081)

Knot's only timeout before this was a **total wall-clock budget** — it
measures *total work*, not *stall*. A session hung on an external call
(deadlocked process, command waiting on stdin, stalled provider) stays
alive but silent, and the rig sat until the budget ran out — potentially
tens of minutes on a profile with a large timeout — before the restart
it triggered carried a nudge that never told the agent *why* it was
stopped. The inverse also bit: a healthy session doing many quick tool
calls could be killed by the budget even though nothing was stuck.

- **Inactivity watchdog** — a rig-global `inactivity-timeout-seconds`
  (`rig/.workspace-agent-config.yaml`, default **300**, `0` disables;
  loaded at startup — restart Knot after editing). Detection is
  byte-level: any byte on the child's stdout/stderr resets the timer.
  A healthy long-running command keeps emitting (pi streams tool
  output, throttled at 100 ms), so it never trips the watchdog; a
  hung command or a stalled provider goes quiet and is killed at the
  window. The two timers are orthogonal: inactivity bounds *silence*,
  the profile's total budget bounds *work*.
- **Cause-accurate error + restart note** — the kill produces
  `PortError::AgentInactivity` (resumable; the one error that may
  retry *without* a session ID — a fresh restart is safe because knots
  are idempotent). The session-resume loop re-enters the same session
  (`--session-id` when one was captured, fresh otherwise) and appends
  a cause-specific note instead of the generic final-response request:
  *“Your last call blocked for more than {N} seconds with no output,
  so your previous turn was stopped. If you have a long-running task,
  ensure it emits a progress update at least once within the
  {window}-second window (e.g. run it in the background and poll its
  output, or stream the output). Continue from where you left off and
  produce your final response when done.”*
- **Blocked-call identification** — under `pi-json`, the accumulated
  stream is scanned after a kill and the last `tool_execution_start`
  with no matching end is named in the error and the loom-log entry
  (e.g. `bash("npm run build")`). Best-effort — the note works
  without it.
- **Loom-log observability** — each stall is recorded as an
  `AgentInactivity` loom entry: attempt, silent seconds, window,
  session ID (empty when none was captured), blocked call. Story on
  success: `KnotProcessing → AgentInactivity → SessionResumed →
  KnotCompleted → StrandProcessed`; on exhaustion:
  `… → AgentInactivity → KnotFailed → StrandProcessed(error)` with a
  `TimeoutExceeded` rig-log entry and **no tie-off write** — the same
  outcome family as a total timeout (a deadline did fire, and the
  tie-off is agent output, which a stalled session never produced).
- **Default adapter for new rigs is now `pi-json`** — the compiled
  default flips from `pi-stdio` to `pi-json`, so fresh rigs get
  session IDs, token usage, and inactivity restart with blocked-call
  identification. **Existing rigs are unaffected**: an explicit
  `agent-adapter` in `.workspace-agent-config.yaml` wins over the
  default. Under `pi-stdio`, inactivity detection still works, but
  restarts after a stall are fresh sessions (no `--session-id`) and
  the blocked call is not named.
- **Total-timeout semantics unchanged** — the profile `timeout`
  budget, the per-attempt deadline, the retry bounds (10 retries,
  10-second delay, 5-second minimum remaining), and the
  `TimeoutExceeded` rig-log behaviour are all as before. When both
  deadlines elapse, inactivity wins — it is the more specific
  diagnosis.

No change to profile, knot, loom, tie-off, or event formats. The new
loom-log event variant degrades gracefully: older binaries skip an
`AgentInactivity` line with a warning, and 0.40.0 reads old logs
unchanged. **No rig-document migration required** —
`inactivity-timeout-seconds` is an additive config key; old files
parse with the 300 default.

## v0.38.0 — 2026-08-26

### Feature — Context Overflow: Compact and Continue, with Loom-Log Visibility (Plan 079)

When a knot's pi session hits the model's context limit, Knot no longer
burns all 10 session-resume retries against the same over-full context
— each re-entry overflowed again, and every retry clocked up against
the budget. Instead, **pi's own built-in compaction** handles the
overflow in-process (compact-and-continue), Knot makes every
compaction visible, and a terminal overflow fails fast.

- **Compaction enabled for rig sessions** — a project-level
  `.pi/settings.json` (`{"compaction": {"enabled": true}}`) overrides
  the global setting; it applies only to rig sessions in that
  directory (knot spawns pi inheriting the rig project's CWD).
  `knot-init` seeds the file at rig initialisation (create-if-absent,
  never overwrites an existing settings file). With compaction on, pi
  proactively compacts before the hard limit (16k reserved by
  default) and, when the model rejects an over-full context, compacts
  and auto-retries the prompt in-process — recovery is once per user
  message, so every session-resume re-entry gets a fresh recovery
  chance.
- **`ContextCompacted` loom-log entries** — one entry per successful
  compaction observed in an invocation's JSON stream: `reason`
  (`"overflow"` = the context limit was hit — the entries to count
  when narrowing prompt scope; `"threshold"` = proactive),
  `tokens_before` (pre-compaction size), `session_id`, and `attempt`
  (1 = first attempt, 2 = first retry). The entries mark context
  pressure so the prompt and strand scope can be narrowed; the append
  is best-effort — observability never fails a strand.
- **`ContextLimitReached` fail-fast** — when the stream shows a
  terminal overflow (recovery ran — `willRetry: true` — and the
  context still does not fit — `willRetry: false`), the strand fails
  immediately with `context limit reached: …` (pi's message when
  present). The error is **not resumable**: no session-resume
  retries, no clock-up, a `Failed` tie-off with the message, and **no**
  rig-log timeout (no deadline was exceeded — the same class as plan
  077). A compaction that failed *without* ever running (missing
  model/auth, transient summarisation error) is not fail-fast: the
  nudge loop keeps its job, and the new user message gives pi a fresh
  recovery attempt.
- **No new budget mechanism** — compaction happens inside pi within
  the existing per-attempt timeout and the retry loop's
  `MIN_REMAINING_SECS` bail; the profile timeout still caps strand
  wall time.

No change to profile, knot, loom, tie-off, or event formats. The new
loom-log event variant degrades gracefully: older binaries skip a
`ContextCompacted` line with a warning, and 0.38.0 reads old logs
unchanged. **No rig-document migration required.**

## v0.37.2 — 2026-08-26

### Feature — Final-Response Request on Abrupt Turn-End (Plan 078)

When a pi session ends its turn abruptly — exit code 0 but **no final
response** (plan 077's scenario) — and a session ID was captured, Knot
no longer gives up: it **re-enters the same session** (`--session-id`)
and requests the tie-off, re-sending the original prompt with the
final-response request appended — *“Please produce your final response,
or continue if you have not finished.”* — one nudge for **all** session
resumes (“continue if you have not finished” covers the mid-stream
case; “produce your final response” covers the abrupt-stop case).

- **Transparent recovery** — a non-empty follow-up response is the
  tie-off: normal `Produced` path (tie-off section, `KnotCompleted`,
  event dispatch, late removal, git commit). The strand succeeds as if
  the first attempt had worked. The loom-log tells the whole story with
  existing events: `KnotProcessing` → `KnotEmptyResponse(1)` →
  `SessionResumed(1)` → `KnotCompleted` (nudge worked) or
  `KnotFailed` (exhausted).
- **Bounded by the existing loop** — up to 10 retries with 10-second
  delays, and the profile's overall timeout budget still bounds timed
  profiles (`MIN_REMAINING_SECS = 5s` bail; the budget, not the retry
  cap, is the primary bound for timed profiles).
- **Cause-accurate exhaustion errors** — when retries or the budget run
  out, the terminal error reflects the *last* failure: exhausted empty
  responses → `no final response: agent returned empty response after
  11 attempts (session resume exhausted)` (failed tie-off, **no**
  rig-log entry); a genuine timeout as the last failure → `Timeout`
  (`TimeoutSkipped`, rig-log `TimeoutExceeded`); budget bail-out →
  `Timeout` as before (`overall timeout budget exhausted after N
  attempt(s)`).
- **No session ID → no re-entry** — stdio adapter or unparseable
  output: the 077 terminal failure stands after the first attempt
  (`no session id — cannot request final response`).
- **Composes with event enforcement (059)** — a nudged response flows
  through the normal success path, including the `KnotEventsMissing`
  follow-up if events were expected but still missing.

No change to event enforcement, mid-stream retry gating, adapters, or
document formats. **No rig-document migration required.**

## v0.37.1 — 2026-08-26

### Fix — Empty Response Is Not a Timeout (Plan 077)

When a pi session ends its turn abruptly — exit code 0 but **no final
response** (e.g. `agent_end` carries only intermediate `toolUse`
messages, or the provider stopped generating) — Knot previously
reported a **timeout**: a spurious rig-log `TimeoutExceeded` event and
no tie-off record, with the work silently lost.

An empty response is now its own error
(`no final response: agent returned empty response`), semantically
distinct from a deadline breach:

- **Failed tie-off written** — the tie-off file gains a `failed`
  section (`Processing failed: no final response: …`), so the terminal
  state is visible in the tie-off and in state (`knot status failed`)
  instead of a skipped write.
- **No spurious rig-log entry** — `TimeoutExceeded` now strictly means
  "the agent session exceeded the profile timeout"; an abrupt turn-end
  does not touch the rig-log.
- **Loom-log** — `KnotProcessing`, `KnotEmptyResponse`, `KnotFailed`
  (error carries `no final response: …`), `StrandProcessed`.
- **Genuine timeouts unchanged** — the adapter's SIGKILL timeout and
  the retry loop's overall budget exhaustion remain `TimeoutExceeded`
  rig-log events.

No retry or session re-entry yet — plan 078 builds on the new error
variant to make the situation recoverable. **No document or format
change.** No migration required.

## v0.37.0 — 2026-08-25

### Fix — Queue Entry Identity Self-Heal: Filename Is the Event ID (Plan 075)

The filename stem of a queued event file (`tie-offs/<rig>/events/{id}.json`)
is now the queue entry's **identity**, and the queue self-heals when a
file's JSON `id` drifts from its name:

- **Self-heal on scan** — on every scan, a file whose JSON `id` differs
  from its filename stem is repaired in place (atomic temp→rename rewrite
  with `id := stem`) and one warning is logged to the service log per
  repaired file:
  `[queue] repaired event file {name}: id {old} -> {stem} (filename is
  the queue identity)`. The repair is idempotent — after the first scan
  that touches the file, name == id and no further rewrites occur.
  `queued_at` and all other fields are preserved.
- **Renaming reorders the FIFO (now supported)** — FIFO order is
  filename sort, so renaming a queued event's file (e.g. to an earlier
  `{timestamp}-{rand}` name) moves it within the queue. This is the
  supported way to front a queued event (e.g. a manual rectify); the
  queue repairs the internal id on the next scan.
- **No more silent wedges or panics** — a head that vanishes between
  scan and read (concurrent late-removal or `knot step`) is logged with
  the file name (`[queue] head {name}.json vanished before read
  (concurrent removal?)`) and handled gracefully: `front()` returns
  `None`, `pop()` rescans once and retries instead of panicking.

This closes the 2026-08-25 borrow-my-stuff incident class: a renamed
queue file previously made the head unresolvable (`front()` swallowed
the read failure and returned `None` — the rig went idle with a
non-empty queue and no log line), a restart duplicated the event, and
late removal orphaned the renamed file. All three paths are now pinned
by unit tests and full-composition incident-reproduction tests
(`tests/queue_identity.rs`).

**No document or format change.** The queue file schema is unchanged;
queues containing renamed or hand-edited event files self-heal on the
first scan of the new binary. No migration required.

### Fix — Consumer Persistent Wake: No Lost Queue Notifications (Plan 076)

The event-queue wake is now persistent **by construction**:

- **Armed-at-call `notified()`** — `StrandEventQueue::notified()`
  registers its `tokio::sync::Notify` permit at call time (armed, not
  lazy), and the contract is documented on the port: a signal sent after
  the call is guaranteed to wake an await of the returned future, even
  if the await has not started.
- **Arm-before-check consumer loops** — the service loop (`next_event`)
  and `knot step`'s head wait create the armed wait *before* re-checking
  `front()`: a push before the arm is visible to the fresh disk scan, a
  push after the arm is captured by the permit. A front hit or a timeout
  drops the armed future (harmless); the next iteration re-arms.

**The wake guarantee:** a queued event always wakes the processor; the
only empty-queue state is a genuinely empty `events/` directory.

**Internal change, no document change.** Notably, the pinned tokio
1.52.3 already stores a `notify_one` permit when no waiter is
registered (verified in the tokio source and by running the new tests
against the pre-fix code — all pass), so no wake was being lost in the
field on this tokio version: this release removes that latent
dependency and makes the guarantee explicit in Knot's own code,
pinned by unit tests, independent of tokio version details. No
migration required.

## v0.36.0 — 2026-08-24

### Feature — Thinking Level: Alias Default with Profile Override (Plan 074)

Profiles and model-registry aliases can now set a reasoning effort — a
**thinking level** — for the pi invocation. Two new **optional**
fields, both named `thinking-level` (values: `off | minimal | low |
medium | high | xhigh`):

```yaml
# rig/models.yml — per-alias default
models:
  frontier:
    provider: anthropic
    model: claude-sonnet-4-20250514
    thinking-level: high
```

```yaml
# rig/profiles/analyst.md — profile override wins
---
name: analyst
model-ref: frontier
thinking-level: xhigh
---
```

- **Resolution (effective level):** a `model-ref` profile resolves to
  its own `thinking-level`, else the alias's; a direct-spec profile
  (`provider` + `model`) uses its own value only — the registry is not
  consulted.
- **CLI emission:** the effective level is emitted as
  `--thinking <level>` on the pi invocation (after `--model`, before
  `--tools`) — for **every** effective value, including an explicit
  `off`, which forces off and overrides pi's settings default.
  **Omitting** the field emits no flag at all — pi's own settings
default applies. Absence is **not** `off`.
- **State:** `tie-offs/<rig>/state.json` profile entries gain an
  optional `thinking-level` showing the **effective** level; the key is
  omitted (never `null`) when neither sets one.
- **Validation is lexical only** — pi clamps levels to model
  capability (non-reasoning models run `off`; `xhigh` is honoured only
  where supported). Invalid values are rejected at file-parse time:
  registry → warning + empty registry (never blocks processing);
  profile → hard parse error (`InvalidThinkingLevel`).

**No document migration:** both fields are optional — existing
`models.yml` files and profile files parse unchanged, and without a
`thinking-level` anywhere the pi invocation is byte-identical to
before (no `--thinking` flag). The `knot-update` skill carries the
0.36.0 changelog entry with adoption steps.

### Skills and Docs Updated

- `knot-create` (v5.7.0) — profile frontmatter table + models.yml
  section + override example; state.json example carries the key
- `knot-inspect` (v3.6.0, compat 0.36.0+) — profile listing shows the
  effective `thinking-level` (absent key shown as `default`, mirroring
  the `timeout` convention)
- `knot-init` (v4.3.0) — models.yml seeding note for the optional
  per-alias default
- `knot-update` (v1.12.0, compat 0.36.0+) — 0.36.0 changelog entry
  (field additions, effective-level resolution, off-vs-omission
  asymmetry; migration: none required)
- `src/server.rs` — auto-created `models.yml` template documents the
  new key
- Design reference: `project/design/design-thinking-level.md`
  (resolution hierarchy, CLI emission, off-vs-omission asymmetry,
  lexical-only validation rationale, backward compatibility)

## v0.35.0 — 2026-08-23

### Feature — `knot step`: Single-Event Stepping (Plan 073)

`knot step` processes **exactly one** queued event and exits — the
manual trigger/observation tool for watching a cycle unfold, inspecting
rig state between events, and debugging a misbehaving knot without
letting the service drain the queue back-to-back.

```
knot step [--rig <rig-name>] [--event <event-filename>]
```

- `--event` targets a specific queued event (exact id, `.json`
  optional; unique id prefix; or strand filename — no match lists the
  queue on stderr and exits 1); without it the FIFO head is processed.
- Empty queue → `queue empty`, exit 0. Exit 1 on unknown/ambiguous
  event, no rigs, multiple rigs, or processing failure.
- A step runs the **full service startup** (migration, config seeding,
  rig git init, discovery, watchers, debounce engine, state writer),
  executes the single event, and shuts down with the service-identical
cascade. Events dispatched *during* the step are captured into
  `tie-offs/<rig>/events/` but **not executed**.
- **Logs are not cleared** in step mode — a multi-step session
  accumulates in the loom-logs/rig-log. (Service startups still clear
  them; see v0.34.0.)
- Step rig discovery is stricter than the service: zero `*-rig`
  matches is an error (no implicit `rig/` creation).
- Use `knot step` when the service is **not** running — two processes
  sharing the disk queue can double-read the same event (safe by knot
  idempotency, but wasteful).

### Queue Semantics — Late Removal (At-Least-Once)

The queued event file is no longer removed when the event is *popped*
for processing — it is removed **after the work is done**:

- **On success** — as the last step before the git commit (dispatch,
  tie-off append, loom-log entries, and event enforcement all happen
  first; the commit captures everything, including the removal).
- **On failure/skip** — at the point of failure (consume-on-failure —
  no poison-pill retry loops).

The only window in which an event survives a crash is while its
processing is in flight: a restart re-queues it and the knot re-runs
(safe by knot idempotency). Previously a crash during a long agent run
lost the event silently. The service loop now peeks (`front()`) instead
of popping; the CLI parsing is a pure unit-tested `parse_args`
function. No new dependencies.

**No document migration:** pending events from older versions read
identically (the `events/*.json` schema is unchanged). The `knot-update`
skill carries the 0.35.0 changelog entry.

### Skills and Docs Updated

- `knot-dispatch` (v1.3.0) — new **Stepping: `knot step`** section
  (flags, event resolution, empty-queue behaviour, exit codes, what a
  step does, direct queue write, agent workflow); stale pop-removal and
  debounce-window wording corrected
- `knot-update` (v1.11.0) — 0.35.0 changelog entry (no migration
  required; queue files from older versions read identically)
- `knot-manage` (v1.2.0), `knot-analyst` (v1.4.0), `knot-init`
  (v4.2.0) glossary — queue-removal wording corrected to late removal
- `docs/concepts.md` — new Event Queue section (at-least-once
  semantics, `knot step`)
- PRD `prd-persistent-events.md` — popped-removal goal revised to late
  removal; new Story 6 (manual stepping via the CLI)
- Design reference: `project/design/design-knot-step.md` (late-removal
  ordering contract, crash windows, the step lifecycle, rejected
  minimal-startup alternative)

## v0.34.0 — 2026-08-23

### Per-Run Logs — Loom-Logs and Rig-Log Cleared at Startup (Plan 072)

The operational logs are now **per-run**. On every startup — after
legacy-layout migration, before loom discovery — Knot truncates the
rig-log (`tie-offs/<rig>/.rig-log`) and **every**
`tie-offs/<rig>/<loom-id>/.loom-log`, including orphaned loom dirs
whose loom no longer exists in the rig. Each log always contains
exactly the events of the current run: it starts with the fresh
`KnotRegistered`/`LoomStarted` events and ends with `LoomStopped` at
shutdown.

**Why:** the logs exist for current-run observability (event-watching
knots, `knot-inspect`/`knot-analyst`). The durable audit history lives
in the git-versioned tie-off files. Previously, stale unparseable lines
re-fired a `WARN:` skip on every 5-second state write and every query
— forever — and the logs grew unbounded with residue nothing consumes.

| Artifact | Before | 0.34.0+ |
|---|---|---|
| `.rig-log`, `*/.loom-log` | accumulated across runs | truncated at every startup |
| Tie-off files, `state.json`, `events/`, dispatch dirs | unchanged | unchanged |

**Non-fatal:** a failed clear logs a `WARNING:` and startup proceeds.
Only log files are touched — nothing else is deleted or modified.

**No document format changes:** profiles, knots, looms, and tie-offs
are unaffected. On the first run of 0.34.0, all earlier runs'
`.rig-log`/`.loom-log` content is discarded — intentional (tie-offs
retain the history).

### Skills and Docs Updated

- `knot-update` (v1.10.0) — 0.34.0 changelog entry (no migration
  required; first run discards earlier runs' log content)
- `knot-inspect` (v3.5.0), `knot-analyst` (v1.3.0) — log descriptions
  annotated per-run scope; analyst failure counting now "since the
  last startup"
- `docs/concepts.md` — Logs section rewritten for per-run semantics
- Design reference: `project/design/design-startup-log-clear.md`
  (startup sequence, ordering invariants, what the clear touches)

## v0.31.0 — 2026-08-17

### Breaking — Rig/Project Repository Split (Plan 068)

The rig no longer holds any runtime data. The runtime tree — tie-off
directories, loom-logs, the event queue, the rig-log, and the state
snapshot — moves from `rig/` to `tie-offs/<rig-basename>/` in the
project root (default rig: `tie-offs/rig/`).

| Path | Before | After |
|---|---|---|
| State snapshot | `rig/state.json` | `tie-offs/<rig>/state.json` |
| Tie-off files | `rig/tie-offs/{loom-id}/…` | `tie-offs/<rig>/{loom-id}/…` |
| Loom-log | `rig/tie-offs/{loom-id}/.loom-log` | `tie-offs/<rig>/{loom-id}/.loom-log` |
| Event dispatch dirs | `rig/tie-offs/{loom-id}/{EventId}/` | `tie-offs/<rig>/{loom-id}/{EventId}/` |
| Event queue | `rig/events/` | `tie-offs/<rig>/events/` |
| Rig-log | `rig/.rig-log` | `tie-offs/<rig>/.rig-log` |

**Rig repository:** Knot initialises `rig/.git` at startup (idempotent).
The rig tracks exactly its source (looms, knots, profiles, config) and
is committed **manually by the user**. When the project root is inside a
git repo, Knot appends a marked `rig/` line to the project's
`.gitignore`, and the git versioner unstages `rig/` before every commit
so the rig can never leak into a project commit (gitlink or tracked
leftovers).

**Auto-migration:** on first 0.31.0 startup, legacy runtime files are
moved automatically (`[startup] migrated …` notice). Idempotent;
destination-exists conflicts keep the destination and warn.

**Pre-existing projects (manual step):** if `rig/` was already tracked
by the project git, run the one-time
`git rm -r --cached rig/` + commit to untrack it. Knot logs a warning
and never runs `git rm` itself.

**Watcher caveat:** after migration, dispatch directories that already
contain unprocessed event files are watched at their new path, but the
file watcher does not rescan existing files — touch each unprocessed
event file to re-trigger processing.

**No document format changes:** profiles, knots, and looms are
unaffected. `knot share` is unchanged — the zip now equals exactly the
rig git's tracked content.

### Skills and Docs Updated

- `knot-init` (v4.0.0) — running-check path moves to
  `tie-offs/<rig>/state.json`; new Rig Repository section
- `knot-glossary` — new **Runtime Tree** term; all paths re-rooted
- `knot-manage` (v1.1.0) — two-repo review workflow; post-migration
  watcher caveat
- `knot-inspect` (v3.3.0), `knot-analyst` (v1.2.0), `knot-dispatch`
  (v1.1.0), `knot-create` (v5.5.0), `knot-design` (v1.5.0),
  `knot-abstractions` (v1.2.0) — path references, diagrams, and
  quick-reference commands updated
- `knot-update` — 0.31.0 changelog entry with migration + verification
  instructions
- `docs/configuration/rig-structure.md` — new directory tree, Rig
  Repository and Runtime Tree section; `docs/concepts.md`,
  `docs/getting-started.md`, `docs/troubleshooting.md`,
  `docs/workflows/*`, `docs/configuration/knots.md`,
  `docs/configuration/profiles.md`, `README.md` — path references

## v0.30.1 — 2026-07-24

### Bugfix — Startup ordering: persisted events processed after loom discovery

After restarting Knot with events in the queue, the process-strand loop
processed them before `DiscoverLooms` had run, causing
`loom 'X-loom' not found` errors for every persisted event. Fixed by
deferring the process-strand loop until after `run_startup()` completes.

### Feature — `knot-manage` skill

New skill for retrospective review of completed rig work. Examines
tie-off files, assesses output quality, traces producer→consumer
interaction chains, and reviews git commit quality. Complements
`knot-analyst` which focuses on live operational health.

### Feature — `knot-dispatch` skill

New skill for triggering knots into action. Creates or touches strand
files, dispatches events manually, and follows the full event pipeline
from strand creation to tie-off completion.

### Documentation

- `getting-started.md` — updated skill installation to include all 8
  skills with verification step
- `concepts.md` — new "Agent Skills" section
- `design-guide.md` — references `knot-design` skill
- `troubleshooting.md` — new section on diagnostic skills
- `workflows/` — references to `knot-dispatch` and `knot-manage`
- `README.md` — expanded Quick Start with workflow steps

## v0.30.0 — 2026-07-21

### Feature — Persistent Event Queue (Disk-Backed)

Strand events are now persisted to `rig/events/{id}.json` on disk
instead of held in memory. Events survive process restarts (Ctrl+C,
crashes) and are restored before processing resumes.

**How it works:**

- Every event pushed is written atomically (temp file → rename) to
  `rig/events/`
- On startup, `rig/events/*.json` files are scanned and re-queued
  before the debounce engine starts
- When an event is processed (popped), its file is removed from disk
- The disk is the source of truth — editing a pending event file on
  disk is honoured when the event is processed
- Malformed JSON files are skipped with a warning; non-`.json` files
  are silently ignored

**Architecture changes:**

- `InspectQueue<Option<TimestampedStrandEvent>>` replaced by
  `StrandEventQueue` trait with `DiskBackedEventQueue` as the primary
  implementation
- `PendingEvent` domain model with unique IDs
  (`{unix_timestamp_ms}-{4-hex-chars}`)
- `PendingEventOrShutdown` enum replaces `Option<T>` for the shutdown
  sentinel
- Dedup key preserved: `(strand_path, loom_id, knot_id, kind)`

**Migration:**

No migration needed. On first start, `rig/events/` is created
automatically. Existing rigs continue working without changes.

### New: Events Directory glossary term

The Knot glossary now documents the `rig/events/` directory layout
and purpose.

### Testing

- 8 new integration tests in `tests/persistent_queue.rs` covering
  full persistence cycle, restart survival, malformed file handling,
  queue deletion, and on-disk modification
- Full suite: 725 unit + 409 integration = 1,134 tests passing

## v0.22.1 — 2026-07-03

### Bugfix

- Fixed flaky `execute_timeout_regression` test under `--test-threads=4` (ETXTBSY)

### Testing

- Completed integration test migration (phases 0–11). Application tests use mock ports, adapter tests use real I/O with `tempfile`, composition smoke tests verify full wiring. `TEST_MUTEX`, process-global env vars, and `KNOT_TEST_CLI_PATH` eliminated. Lib tests run in ~1.1s.

## v0.22.0 — 2026-07-01

### Breaking Change — Flat tie-off paths

Tie-off paths changed from `rig/tie-offs/{loom-id}/{knot-name}/{strand}.output` to `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`. The intermediate knot subdirectory is removed. Tie-offs are now one file per knot with append-mode writes.

Migration: Update any scripts or tooling that reference the old path structure.

### Feature — Strand queue visibility

`rig/state.json` now includes a `strand_queue` array showing all pending strand events with file path, loom/knot IDs, event type, and queued timestamp.

### Bugfix

- Fixed `spawn_blocking` for `ProcessStrand execute()` — ensures graceful shutdown on Ctrl+C

## v0.21.0 — 2026-07-01

### Feature — Final response filtering in Pi JSON adapter

`PiJsonAgentRunner` now extracts only the agent's final response text. When Pi uses tools, intermediate messages with `stopReason: "toolUse"` are excluded; only `"stop"` and `"length"` responses produce output. This prevents tool-use artifacts from appearing in tie-off files.

## v0.20.3 — 2026-06-29

### Refactor

- Extracted `usecases.rs` into isolated modules (`loom/`, `query/`, `session_resume/`). Pure structural refactor — zero behaviour change.

## v0.20.1 — 2026-06-29

### Bugfix

- Removed unused imports from process_strand test modules

## v0.20.0 — 2026-06-28

### Feature — Session resume on invocation failure

Automatically resume Pi sessions from where they left off after invocation failure (timeout, network error). Uses `--session-id` for up to 10 retries with 10-second delays between attempts. Profile timeout budget is respected — retries stop when insufficient time remains. Each retry appends "please continue" to the session.

## v0.19.0 — 2026-06-27

### Feature — JSON-based agent adapter

New `agent_adapter` enum in `.workspace-agent-config.yaml` replaces `cli_path`/`cli_args`. Supports `pi-stdio` (default, reads stdout) and `pi-json` (parses JSON-L for session IDs and token usage). `run_startup()` auto-creates the config file on first boot.

## v0.18.1 — 2026-06-26

### Bugfix

- Fixed `unwatch()` removing all watcher entries for a path when only a single knot's entry should be removed. Broke shared strand directory scenarios where multiple knots watch the same directory.

## v0.18.0 — 2026-06-24

### Breaking Change — Prompt text moved to markdown body

Profile and knot files no longer embed prompt text in YAML frontmatter. The plain text after the `---` separator is now the prompt content.

Frontmatter retains only structural metadata (name, provider, model, tools, timeout for profiles; name, agent-profile-ref, strand-dir, git-versioned for knots).

## v0.17.0 — 2026-06-24

### Feature — Strand missing file handling

Known temp files (e.g. macOS `sed -i` temp files) are silently skipped. Unknown missing files produce `StrandSkipped` events in the loom-log instead of spurious "File not found" errors.

## v0.16.0 — 2026-06-22

### Feature — Tie-off context extraction for deleted files

When a strand is deleted, Knot now parses the tie-off file and injects the last N per-strand entries into the agent prompt (replacing the `@file` reference that would fail on deleted files).

## v0.15.0 — 2026-06-20

### Breaking Change — Removed `input-bundling` from knot frontmatter

The `input-bundling` property was removed from knot YAML frontmatter. It had no runtime effect — only `full-file` ever shipped and is always the behaviour. Knot files that still contain `input-bundling` parse with a warning.

## v0.14.0 — 2026-06-19

### Feature — All text files accepted as strands

Knots now process any text file (`.rs`, `.json`, `.py`, `.txt`, etc.) — not just `.md`. Binary files are detected (null-byte heuristic on first 8KB) and silently skipped with `StrandIgnored` in the loom-log.

## v0.13.0 — 2026-06-19

### Breaking Change — HTTP interface removed

The Axum HTTP server was removed entirely. All state observation is now through `rig/state.json`, written atomically every 5 seconds. `GET /health`, `GET /looms`, `GET /profiles`, and all other HTTP endpoints no longer exist. Skills and tools read `rig/state.json` directly.

## v0.12.0 — 2026-06-17

### Feature — Explicit Pi session titles

Each agent session gets a unique, descriptive title derived from knot ID and strand filename (e.g. `plan-architect triggered by Modified on 004-manifest-resources.md`).

### Core Features

Knot is a local agent orchestration system that watches directories for
file changes and triggers AI agent sessions. Key capabilities:

- **File-first configuration** — All configuration is `.md` files with
  YAML frontmatter. Git-trackable, diff-visible.
- **Auto-discovery** — Looms (`*-loom/` directories), knots (`.md`
  files in looms), and profiles (`rig/profiles/*.md`) are discovered
  automatically via file watching.
- **Agent profiles** — Define which LLM provider, model, tools, and
  system prompt to use. Profiles are read fresh from disk at processing
  time.
- **Knot processing** — Goal-seeking agents that read strands (input
  files), inspect current state, and apply minimal changes to reach a
  goal. Idempotent by design.
- **Tie-off output** — Append-only output files at
  `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`.
- **Git versioning** — Automatic commits after each tie-off write
  (opt-out per-knot with `git-versioned: false`).
- **Session resume** — Automatic retry of failed agent sessions (up to
  10 retries, 10s delay).
- **State file** — `rig/state.json` updated every 5 seconds with looms,
  knots, profiles, and strand queue.
- **Activity logging** — Per-loom activity logs and a rig-wide
  operational log (`rig/.rig-log`) in JSONL format.
- **Rig switching** — Multiple rigs per project, with packaging for
  sharing.
- **Debounced event processing** — File events are debounced to avoid
  triggering on partial writes.
- **Graceful shutdown** — Cooperative cascade shutdown that drains
  pending events.
- **Configurable timeouts** — Per-profile session timeouts with
  `TimeoutExceeded` event logging.
