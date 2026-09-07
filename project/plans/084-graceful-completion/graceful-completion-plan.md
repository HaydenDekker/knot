# Plan 084: Graceful Completion — Steer the Session to Wrap Up Before Context Runs Out (pi-rpc Runner)

## Related Plans

Builds on [079 Context Overflow — Compact and Continue](../079-context-overflow-compact-and-continue/context-overflow-compact-and-continue-plan.md)
(compaction records + `ContextCompacted` loom-log visibility +
`PortError::ContextLimitReached` — the *reactive* half of context
pressure; this plan adds the *proactive* half),
[080 Overflow Fail Fast](../080-overflow-error-fail-fast/overflow-error-fail-fast-plan.md)
(overflow classification; its terminal path stays untouched as the
safety net),
[081 Inactivity Timeout](../081-inactivity-timeout/inactivity-timeout-plan.md)
(the reader/watchdog/`LiveOutput` machinery in the adapters that the
new runner reuses verbatim),
[078 Final-Response Request](../078-final-response-request/final-response-request-plan.md)
(the greppable-const convention for user-facing agent nudge text),
[074 Thinking Level](../074-thinking-level-hierarchy/thinking-level-hierarchy-plan.md)
(the alias-config pattern in `rig/models.yml` that
`ctx-wrap-up-limit` follows), and
[069 Model Aliases](../069-model-aliases/model-aliases-plan.md)
(the registry itself).

## Problem

A knot session that runs out of context ends badly. Today the
sequence is: the context fills, pi compacts (losing the agent's
fine-grained working state), the session keeps going on the summary,
and if even the compacted context does not fit, the run dies with
`ContextLimitReached` — **no tie-off, no commit, no notes**: the
working tree holds half-finished work with no record of what was done,
what remains, or where the agent left off. Even in the milder
compaction case the agent continues on degraded memory and the tie-off
it eventually writes is unreliable.

Knot has no way to *intervene* while a session is running. The
existing runners (`pi-json`, `pi-stdio`) spawn `pi -p` (print mode):
one prompt in, events out, process exits. The prompt is written to
stdin and stdin is **closed** — there is no channel to tell the agent
anything mid-run. The `ContextProvider` seam cannot help either: it
decorates the prompt *before* the run starts.

What we want: when the session context crosses a **configurable
threshold set below the model's effective compaction point**, Knot
injects a message *into the running session* telling the agent to stop
starting new work, commit everything complete, update its progress
documentation, leave notes on what is incomplete, and produce its
final tie-off response — the work ends **gracefully** instead of being
torn off by the context limit.

pi supports exactly this over its **RPC mode** (`pi --mode rpc`,
pi's `docs/rpc.md`): a JSONL command protocol on stdin while events
stream on stdout. Two RPC features are the whole mechanism:

- `steer` — "Queue a steering message while the agent is running. It
  is delivered after the current assistant turn finishes executing its
  tool calls, **before the next LLM call**." The agent is never
  interrupted mid-tool-call; the wrap-up instruction arrives at the
  next turn boundary with full working state intact.
- `get_session_stats` — returns `contextUsage: { tokens, contextWindow,
  percent }` — "the actual current context-window estimate **used for
  compaction and footer display**". This is pi's own number, so the
  threshold is measured on the same scale pi uses to decide when to
  compact.

Print/json mode cannot do either (no input protocol; JSON-mode stdout
parsing in knot is post-hoc over a buffer). So: a **new runner**.

## Why the RPC runner, and what the delta actually is

The new `pi-rpc` runner implements the existing `AgentRunner` port —
`ProcessStrand`, `session_resume`, tie-off writing, and event
dispatch are untouched. Compared to `PiJsonAgentRunner`, most of the
body is reused verbatim:

| Piece | `pi-rpc` runner |
|---|---|
| Spawn (own process group, piped stdio) | unchanged |
| stdout/stderr reader threads + `LiveOutput` buffers | unchanged (byte-level activity still feeds the inactivity watchdog) |
| Watchdog (`KillReason::{Inactivity, Total}`, kill `-pgid`, 2× deadline join guard) | unchanged |
| Event vocabulary (`session`, `compaction_end`, `agent_end`, …) | identical — the RPC event stream is a **superset** of the json stream; the same `parse_json_line` shapes apply |
| Prompt assembly (`build_prompt_with_context`: profile prompt + instructions + trigger line) | unchanged, wrapped in a `{"type":"prompt",...}` command |
| `--name <title>` / `@{strand}` injection via `extra_args` | unchanged (rpc honors the same CLI startup args) |

Genuinely new: stdin stays open with a locked line writer; the prompt
is sent as a command; the stdout lines are additionally parsed
**incrementally** (the json runner only parses after exit) by a small
session driver that polls context size and writes the steer; the
process is torn down after `agent_end` (an rpc process idles waiting
for commands instead of exiting).

This plan ships `pi-rpc` **side-by-side** with `pi-json` (opt-in via
`agent-adapter: pi-rpc`). Removing `pi-json` is the **explicitly
deferred next change** once the rpc path has proven itself in a real
rig.

## Target — behavioural contract

### Configuration (per model alias)

`rig/models.yml`:

```yaml
models:
  worker:
    provider: anthropic
    model: claude-sonnet-4-20250514
    ctx-wrap-up-limit: 80000   # optional; tokens; absent or 0 = feature off
```

- `ModelRef` gains `ctx_wrap_up_limit: Option<u64>`
  (`#[serde(rename = "ctx-wrap-up-limit")]`), parsed via the
  raw-string pattern used by `thinking-level` so an invalid value
  produces `ModelRegistryError::InvalidWrapUpLimit { alias, value }`
  (registry degrades to warning + empty registry, as every other
  malformed entry does today).
- `0` means disabled (mirrors the `inactivity-timeout-seconds: 0`
  convention): `resolve_for_knot` maps it to `None`.
- `AgentConfig` gains `ctx_wrap_up_limit: Option<u64>` (serialized
  name `ctx-wrap-up-limit`, omitted when `None`), populated from the
  resolved alias — the same `profile.or(alias)`-style flow as
  `thinking-level`, except the profile has **no** field of its own:
  v1 is alias-level only (the spec). A direct-spec profile (no
  `model-ref`) has no alias, therefore no limit, feature off.
- The limit is meaningful only on `pi-rpc` runs: the registry is
  alias-scoped, not adapter-scoped, so when a knot resolves a limit
  under a non-rpc adapter, the runner logs a one-shot warning
  (`ctx-wrap-up-limit configured but adapter cannot steer mid-run —
  requires agent-adapter: pi-rpc`).
- `models.yml` is already read fresh per strand — the limit is live on
  the next edit, no restart. No hot-reload concern, no migration
  (additive key).
- The `models_template` in `server.rs` gains a commented example of
  the key.

### Threshold semantics

Let `L` = `ctx-wrap-up-limit`, `W` = model `contextWindow`, `R` = pi's
`reserveTokens` (default 16384). pi's auto-compaction fires at
`W − R`; `L` must sit **below `W − R`**, not just below `W`, to win the
race (on a 100k-window model with the default reserve, `L = 80000`
would never fire — pi compacts at ~84k first).

Knot cannot know `R` reliably (pi settings may override it), but the
first `get_session_stats` response carries `contextWindow`, so the
driver checks once per run: if `L >= W − PI_DEFAULT_RESERVE_TOKENS`
(new const `= 16384`) and pi compaction is enabled (existing
`effective_pi_compaction_enabled` helper), `WARN` once — "pi may
compact before the wrap-up limit is reached; lower
`ctx-wrap-up-limit` or raise pi's `reserveTokens`". Warning only,
never blocking. The existing 079/080 overflow paths stay exactly as
they are and continue to serve as the safety net when the limit is
misconfigured or when a single tool result jumps the context past the
limit between samples.

### Monitoring (session driver, `pi-rpc` runner)

With `ctx-wrap-up-limit` set, the driver samples **after every
`turn_end`** (a turn = one assistant message + its tool calls — the
same granularity at which the steer can land):

1. On `turn_end` → write
   `{"id":"stats-N","type":"get_session_stats"}` to stdin (one
   in-flight poll at a time).
2. On the matching `{"type":"response","command":"get_session_stats",
   "success":true,"data":{...}}` line → read `data.contextUsage.tokens`.
   `contextUsage` or `tokens` `null` (documented pi behaviour: null
   immediately after a compaction until a fresh assistant response) →
   skip the sample.
3. If `tokens >= L` and not yet fired → **steer** (below) and mark
   fired.

One poll round-trip per turn is a local stdin write + stdout line —
negligible. Without a limit the driver only watches for `agent_end`
(teardown) — zero extra protocol traffic, and the rpc runner behaves
as the json runner with a different handshake.

**Granularity note (documented):** a steer issued after the sample of
turn N is delivered before turn N+1's *next* LLM call — in the worst
case the agent completes one more LLM call after the threshold is
crossed. One call of drift is harmless when `L` sits well below
`W − R`, and it is why the steer is never "urgent — stop mid-call".

### The wrap-up message (`pi-rpc` only)

One greppable const, same convention as `FINAL_RESPONSE_REQUEST` /
`INACTIVITY_RESTART_NOTE` (user-facing agent text — keep it
greppable):

```rust
/// The context wrap-up steer (plan 084), sent once per run when the
/// session context crosses the alias's `ctx-wrap-up-limit`. Tells the
/// agent to stop starting new work, commit complete work, update
/// progress, note the incomplete, and produce its final tie-off.
/// `{context_tokens}` / `{limit}` are substituted by the driver.
const WRAP_UP_NOTE: &str = "Your session context has reached \
    {context_tokens} tokens, over the {limit}-token wrap-up limit for \
    this model. Stop starting new work and finish gracefully:\n\
    1. Commit all complete work (git add + git commit, descriptive message).\n\
    2. Update your progress documentation to reflect the current state.\n\
    3. Note what is incomplete and exactly where you left off (files, \
    next steps) so the work can be resumed.\n\
    4. Produce your final tie-off response: what was done, what remains, \
    how to resume.\n\
    Reconstruct the current state from the filesystem (git status/diff, \
    progress docs, recently modified files) — do not rely on memory of \
    earlier work.";
```

Design points baked into the text:

- **"Reconstruct from the filesystem, not memory"** — the wrap-up may
  fire around a compaction, and the session-resume machinery can put
  it on a re-entered session; the filesystem (git, progress docs) is
  the rig's source of truth and survives both.
- **"Commit all complete work"** is the agent's own `git commit` (it
  has the bash tool); Knot's existing per-knot git-versioning commit
  still runs afterwards and captures the tie-off and any remaining
  artifacts.
- The **final tie-off is the run's normal final response** — the
  steer does not end the run; the agent winds down inside the same
  session and the existing tie-off path records whatever it produces.

**Fire-once:** after a steer, context is still ≥ `L`, so the driver
never steers again in that run (`fired` flag; no re-arm on compaction
in v1 — see Notes).

### Teardown

An rpc process stays alive after `agent_end` waiting for more
commands. On `agent_end` the driver closes stdin; if the child has not
exited within a 5s grace window the existing group kill fires.
Classification precedence: **`agent_end_seen` beats a kill reason** —
a SIGKILL during teardown after `agent_end` is a normal exit, not a
`Timeout`. Output assembly continues to reuse the existing post-hoc
`parse_stdout` over the accumulated buffer (session ID, response text
filtered by `stopReason`, compactions, error message — unchanged),
plus the new wrap-up record from the shared monitor state.

### Errors and retries — unchanged surface

- Timeout / inactivity / exit-classification, `PortError` shapes, and
  `TieOffOutcome::derive` are unchanged. The session-resume retry loop
  re-spawns `pi --mode rpc --session-id <id>` via the same
  `extra_args` injection as today (pi's `--session <path|id>` is a
  general CLI flag; the probe list in Phase 6 confirms the exact
  `--session-id` spelling under `--mode rpc` — if rpc rejects it, the
  fallback is the plan-081 precedent: fresh restart, knots are
  idempotent; record the finding in the Implementation Status).
- `queue_update`, `auto_retry_*`, and `compaction_*` events are
  tolerated by the incremental parser (tracked or ignored — never an
  error).

### Observability

- New record + metadata field (`src/application/ports.rs`):

  ```rust
  /// One context wrap-up steer observed in an invocation (plan 084).
  pub struct WrapUpRecord {
      /// `contextUsage.tokens` at the moment the steer was queued.
      pub context_tokens: u64,
      /// The configured `ctx-wrap-up-limit`.
      pub limit: u64,
  }
  // AgentInvocationMetadata gains:
  pub wrap_up: Option<WrapUpRecord>,
  ```

- New loom event (`src/domain/events.rs`), logged best-effort from
  `session_resume` exactly like `ContextCompacted` (attempt
  convention: `KnotEmptyResponse`-style `attempt + 1`):

  ```rust
  /// The session context crossed the alias's `ctx-wrap-up-limit` and
  /// Knot steered the agent to wrap up gracefully (commit, update
  /// progress, note the incomplete, final tie-off). One entry per
  /// invocation at most.
  ContextWrapUpSteered {
      loom_id: LoomId,
      knot_id: KnotId,
      strand_path: StrandPath,
      session_id: String,
      context_tokens: u64,
      limit: u64,
      attempt: u32,
      timestamp: String,
  }
  ```

- Service-log line at the moment of the steer (greppable):
  `[rpc] context wrap-up steer sent (knot=…, context_tokens=…,
  limit=…)`.
- Loom-log success story:
  `KnotProcessing → ContextWrapUpSteered → (ContextCompacted…) →
  KnotCompleted → StrandProcessed`.

## Non-Goals

- **Removing `pi-json`** — explicitly the next change (this plan ships
  the runners side-by-side; the removal flips defaults and deletes
  code once `pi-rpc` earns trust in a real rig).
- In-process `prompt` commands for the 078 nudge / event-enforcement
  follow-up (the live session makes them possible later; v1 keeps
  today's re-spawn with `--session-id`).
- Re-arming the monitor after a compaction (one steer per run in v1).
- Per-profile / per-knot wrap-up limits (alias-level is the spec).
- `state.json` visibility for the limit (the loom event and the
  service log cover observability).
- RPC `abort` as a timeout mechanism (the watchdog keeps SIGKILL-ing
  the process group; `abort` is a later refinement).
- Multi-prompt session reuse (one prompt per process per strand run).
- Changes to `pi-json` / `pi-stdio` behaviour (the feature cannot
  engage there; they gain only the one-shot "limit set but adapter
  cannot steer" warning path).
- `knot-update` migration entry (additive `models.yml` key; new
  adapter value is opt-in).

## Phases

**Status: ⬜ Planned.**

### Phase 0: Failing tests

1. `src/adapters/pi_rpc.rs` (new module; mock CLI scripts via
   `KNOT_TEST_CLI_PATH`, mirroring the pi-json harness). The rpc mock
   is a line-based command server: reads stdin commands, emits events
   on stdout. Script behaviour: on the `prompt` command emit
   `{"type":"session",...}`, `agent_start`, then a configurable
   sequence of `turn_start`/`message_end`/`turn_end` groups, answer
   `get_session_stats` commands with a queued `contextUsage` value,
   answer `steer` with `{"type":"response","command":"steer",
   "success":true}` + `queue_update`, then emit `agent_end` and exit.
   Tests:
   - `execute_rpc_success` — prompt → events → `agent_end`; `Ok` with
     the final response text and `session_id`; child torn down without
     hitting the watchdog.
   - `execute_rpc_no_limit_no_polls` — `ctx_wrap_up_limit: None` →
     mock's captured stdin contains **no** `get_session_stats` and no
     `steer` lines (pure pass-through).
   - `execute_rpc_steers_on_threshold` — limit 80000; first stats
     sample 70000, second 85000 → exactly one `steer` line on stdin
     whose `message` contains the wrap-up text with the substituted
     numbers; `metadata.wrap_up == Some(WrapUpRecord {
     context_tokens: 85000, limit: 80000 })`.
   - `execute_rpc_steers_once` — samples stay ≥ limit for many turns →
     exactly one `steer` line.
   - `execute_rpc_stats_null_skipped` — `contextUsage.tokens: null`
     (post-compaction shape) → no steer, no crash; a later real
     sample still steers.
   - `execute_rpc_limit_below_all_samples` — samples stay under →
     `wrap_up == None`, no steer.
   - `execute_rpc_warns_when_limit_above_compaction_point` — stats
     response `contextWindow: 100000`, limit 90000, compaction enabled
     → stderr warning mentions the effective compaction point (once
     per run).
   - `execute_rpc_timeout_still_timeout` — silent mock + small total →
     `PortError::Timeout` (watchdog regression).
   - `execute_rpc_inactivity_kill` — mock emits session line then
     sleeps → `PortError::AgentInactivity` with session ID (plan-081
     regression on the new transport).
   - `execute_rpc_agent_end_then_idle` — mock emits `agent_end` and
     stays alive → runner tears down via stdin close within the grace
     window and returns `Ok` (teardown path).
   - `execute_rpc_nonzero_exit` → `AgentExecutionFailed` with session
     ID parsed from buffer (unchanged error shapes).
2. `src/domain/value_objects.rs`:
   - `models.yml` round-trips: `ctx-wrap-up-limit` absent → `None`;
     `80000` → `Some`; `0` → stored, `resolve_for_knot` → `None`;
     non-numeric → `InvalidWrapUpLimit { alias, value }` (registry →
     warning + empty); JSON round-trip of `ModelRef`/`AgentConfig`
     (`skip_serializing_if` — absent key, never `null`).
   - `resolve_for_knot`: alias profile inherits the alias limit;
     direct-spec profile → `None`.
3. `src/application/session_resume.rs` (`TestAgentRunner`):
   output carrying `metadata.wrap_up` → `ContextWrapUpSteered` logged
   once with the right fields (mirrors the `log_compactions` tests).
4. `src/application/usecases/process_strand.rs`: happy path with a
   wrap-up-carrying mock output — tie-off `Produced`; loom-log
   `KnotProcessing → ContextWrapUpSteered → KnotCompleted →
   StrandProcessed`; rig-log empty.
5. `tests/` integration (`agent_integration.rs` style, rpc adapter via
   the mock): end-to-end strand → tie-off produced +
   `ContextWrapUpSteered` line in the loom-log file.

### Phase 1: Config

`src/domain/value_objects.rs`:

- `RawModelEntry.ctx_wrap_up_limit: Option<String>` (raw-string pattern
  — precise alias-naming errors); parse `u64`; new
  `ModelRegistryError::InvalidWrapUpLimit { alias, value }` (+ `Display`).
- `ModelRef.ctx_wrap_up_limit: Option<u64>`
  (`rename = "ctx-wrap-up-limit"`, `skip_serializing_if`);
  `from_yaml` wires it; round-trip tests.
- `AgentConfig.ctx_wrap_up_limit: Option<u64>` + constructors/defaults.
- `AgentProfile::resolve_for_knot`: alias → `alias.ctx_wrap_up_limit`
  mapped through `filter(|v| *v > 0)`; direct-spec → `None`.
- `src/server.rs` `models_template`: document the key (commented).

### Phase 2: `pi-rpc` runner skeleton

`src/adapters/pi_rpc.rs` — `PiRpcAgentRunner` implementing
`AgentRunner` (`runner_type() == "pi-rpc"`), mirroring the pi-json
constructors (`with_timeouts`, `with_cli_path_and_timeout(s)`):

- Spawn args: `AgentConfig::build_cli_args()` refactored so the `-p`
  prefix is not baked in (`build_cli_args()` keeps today's output for
  json/stdio; shared core without `-p` + `--mode rpc` for this
  runner). `--name`, `@{strand}`, `--session-id` continue to arrive
  through `extra_args` unchanged.
- Stdin: `Arc<Mutex<BufWriter<ChildStdin>>>` +
  `send_command(serde_json::Value)` (single line + `\n` + flush —
  strict LF framing per pi's rpc docs).
- Reader: new `spawn_line_reader` in `live_output.rs` — reads chunks
  into the `LiveOutput` buffer + activity timer (unchanged watchdog
  inputs) **and** splits complete LF lines into an
  `mpsc::Sender<String>` for the driver (partial last line held in the
  buffer; malformed lines pass through, driver ignores).
- Session driver thread: state machine over the line channel —
  send the `prompt` command first (`build_prompt_with_context`,
  unchanged); on `agent_end` set `agent_end_seen`
  (`Arc<AtomicBool>`), close stdin; on `turn_end` / stats-response
  lines apply the monitor rules (Phase 3; with `ctx_wrap_up_limit:
  None` the monitor is inert).
- Wait/classify: existing `join_all` guard; `agent_end_seen` checked
  **before** kill-reason classification (teardown kill is not a
  `Timeout`); exit-code, error-message, and `parse_stdout` paths reused
  verbatim; `AgentOutput.metadata` gains `wrap_up` from shared state.

### Phase 3: Monitor + steer

- `WRAP_UP_NOTE` const + substitution helper (unit-tested for the
  greppable text and number substitution).
- Driver monitor rules exactly per Target: poll-on-`turn_end`
  (one-in-flight, `id` correlation), null-skip, `tokens >= L &&
  !fired → steer + fired`, `WrapUpRecord` into shared state, one-shot
  `contextWindow`/reserve warning (uses
  `effective_pi_compaction_enabled`).
- One-shot service-log warning when the limit is set but
  `runner_type() != "pi-rpc"` — emitted from the resolve path
  (`process_strand_helpers::resolve_config_and_build` has the runner
  via `ps.agent_runner.runner_type()`; static `AtomicBool` keeps it to
  one line per process).

### Phase 4: Error passthrough + observability

- `src/application/ports.rs`: `WrapUpRecord` +
  `AgentInvocationMetadata.wrap_up` (serde; all constructors/tests
  updated — the field is additive everywhere); unit tests.
- `src/domain/events.rs`: `LoomEvent::ContextWrapUpSteered` + serde
  round-trip.
- `src/application/session_resume.rs`: `log_wrap_up` next to
  `log_compactions` (called on every attempt that produced output —
  the driver records at most one, so it cannot double-log).

### Phase 5: Wiring

`src/server.rs`, `build_app_context`: `AgentAdapter::PiRpc` arm →
`PiRpcAgentRunner` (both `cli_path` and fallback branches; same
`config.agent_timeout` + `rig_config.inactivity_timeout()`),
`runner_type` composition test. **No default flip** — `pi-json` stays
default; rigs opt in with `agent-adapter: pi-rpc`.

### Phase 6: Verify + docs + version

- `cargo test` full suite; `cargo clippy --all-targets` clean (new
  code only).
- **Scratch pi probes** (direct `pi` binary in a temp dir — *not* a
  rig run; per AGENTS.md no knot service or rig execution happens in
  this repo):
  1. `pi --mode rpc` + `prompt` command → event stream; confirm
     whether the process exits on stdin close after `agent_end` or
     must be killed (calibrate the 5s grace; adjust Phase 2 teardown
     if pi exits by itself).
  2. Mid-run `steer` → `queue_update` visible; delivered before the
     next call (the wrap-up contract).
  3. `get_session_stats` response shape; `contextUsage.tokens` null
     immediately after a compaction (confirms the null-skip rule).
  4. `pi --mode rpc --session-id <id>` accepted (retry path); if not,
     apply the documented fresh-restart fallback and record it.
- `docs/configuration/rig-structure.md` — `pi-rpc` row in the adapter
  table (session IDs + compaction + **in-run wrap-up steering**; json
  remains default), `ctx-wrap-up-limit` under `models.yml` (tokens;
  must sit below `contextWindow − reserveTokens`; `0`/absent = off;
  `pi-rpc` only).
- `docs/concepts.md` — graceful-completion paragraph in the
  context-pressure story (079 compaction reactive / 084 steering
  proactive; fire-once; filesystem-reconstruction note).
- `docs/troubleshooting.md` — new row: "session context exhausted
  mid-work; nothing committed" → configure `ctx-wrap-up-limit`
  (`pi-rpc`) so Knot steers the agent to commit + note the incomplete
  before the limit; check `ContextWrapUpSteered` in the loom-log.
- `docs/release-notes.md` — v0.41.0: `pi-rpc` runner (opt-in) +
  context wrap-up steering; `pi-json` unchanged; removal of
  `pi-json` announced for a future version.
- Skills: `knot-update` — **no migration entry** (additive key, opt-in
  adapter). Review `knot-create`/`knot-inspect` for the new
  `agent-adapter` value / `models.yml` key only if they enumerate
  them; deploy any touched sub-skill to `~/.agents/skills-library/`
  with diff verification per AGENTS.md.
- Bump `Cargo.toml` `0.40.0 → 0.41.0`; `cargo install --path .`.
- `project/plans/master-plan.md` — row 82 (added at plan-creation
  time with this commit).
- Follow-up (next change, separate plan): flip the default adapter to
  `pi-rpc`, delete `PiJsonAgentRunner` and the `PiJson` variant,
  record the migration in `knot-update`.

## Notes — design rationale

- **Why steering, not kill-and-re-enter?** The alternative Tier-2
  shape (SIGINT the process on threshold, re-enter with `--session-id`
  and the wrap-up prompt — the plan-081 kill machinery) interrupts a
  possibly mid-tool-call agent and forces a process restart. `steer`
  delivers the same instruction at a turn boundary, in-process, with
  the full live session state — no interrupted writes, no reload, no
  extra spawn. The kill machinery stays reserved for what it already
  does (stalls, budgets).
- **Why `get_session_stats` instead of per-message `usage`?**
  `contextUsage.tokens` is pi's own compaction-scale estimate (system
  prompt and cache included) — the number the threshold must be
  compared against. Per-message `usage.input` is available in both
  transports but mixes provider-specific cache accounting into a
  number that is not the compaction estimate. The round-trip cost is
  one local command per turn.
- **Why per-`turn_end` sampling is enough?** Context grows once per
  model call; a turn contains exactly one assistant message. Sampling
  after each turn bounds the drift to a single LLM call past the
  threshold — harmless when `L < W − R` by design, and the warning
  covers the misconfiguration where it would not be.
- **Why one-shot (no re-arm)?** After the steer the context is still
  above `L`; any re-arm rule needs a new policy (re-arm on
  compaction? on the next turn?). The wrap-up note tells the agent to
  *finish*; a second steer to the same wind-down adds pressure without
  information. Revisit if rigs show sessions that compact and then
  run long again.
- **Why does the note say "reconstruct from the filesystem"?** The
  steer fires late in a session that may have (or is about to)
  compact, and 078/081 re-entries already rely on idempotency + the
  filesystem. Telling the agent to trust `git status`/progress docs
  over memory makes the wrap-up robust to exactly the state the
  trigger implies.
- **Why alias-scoped config for an adapter-level capability?** The
  threshold is a property of the **model** (window size), not of the
  rig — it belongs beside `provider`/`model`/`thinking-level`, and
  the registry's fresh-read gives live tuning (lower the limit, next
  strand uses it). The adapter mismatch is a warning, not an error,
  so a rig can set limits once and flip adapters without editing
  every alias.
- **Why side-by-side first?** The rpc runner changes the process
  lifecycle (stdin lifetime, teardown-after-agent_end) on the
  codepath every strand takes. Shipping it as an opt-in adapter lets
  it earn trust in a real rig (and be probed in a scratch dir first,
  per the repo's no-rig-runs rule) before the json runner is removed.
- **Why is the tie-off still the run's final response?** The steer is
  an instruction, not a terminator — the agent winds down and answers
  within the same run. Zero changes to tie-off writing, event
  dispatch, enforcement, git versioning, or late removal: the
  graceful-completion tie-off flows through the entire pipeline like
  any other run, and the loom event records that it was steered.

## Implementation Status: ✅ Complete (2026-09-07) — released in v0.42.0
