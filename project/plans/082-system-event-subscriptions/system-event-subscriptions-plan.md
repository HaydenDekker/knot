# Plan 082: System Event Subscriptions — Strand Off Any Knot Event, with Wildcard Producers

## Related Plans

Builds on [045 Intent-Based Event Routing](../045-intent-based-event-routing/)
(agent-emitted events, tie-off event blocks, consumer dispatch),
[056 Strand Event URI](../056-strand-event-uri/strand-event-uri-plan.md)
(`strand-dir: event:<producer>:<EventId>` grammar),
[058 Loom-Level Events](../058-loom-level-events/loom-level-events-plan.md)
(`EventSubscription`, `resolve_for_producer`, `-loom` suffix precedence),
[ADR-012 Event Placement Per Consumer](../../adrs/adr-012-event-placement-per-consumer.md)
(event files in the consumer loom's `{event-id}/` dir — unchanged),
[077 Empty Response](../077-empty-response-not-timeout/empty-response-not-timeout-plan.md),
[079/080 Context Overflow](../079-context-overflow-compact-and-continue/), and
[081 Inactivity Timeout](../081-inactivity-timeout/inactivity-timeout-plan.md)
(the failure-class errors this plan makes subscribable).

## Problem

Today a knot's `strand-dir` can subscribe to **agent-emitted events only**
(`event:<producer>:<EventId>` matches `AgentEvent` blocks the producer wrote
in its tie-off). Everything else the system knows about a run — success,
failure, timeout, inactivity stall, context overflow, empty response — is
written to the loom-log / rig-log / a failed tie-off and **stops there**.
There is no way to trigger a recovery, reporting, or janitor knot off a
knot *outcome or exception*: a knot that times out or fails just leaves a
tie-off behind, and nothing can react.

The event vocabulary already exists — the full `LoomEvent` (18 variants) and
`RigLogEvent` (2 variants) catalog — it is just not *dispatchable*. And the
dispatch machinery (consumer matching, per-consumer event files, watches,
debounce, queue) already exists and works; only the **emitter** for
system-produced events and a **wildcard producer** are missing.

## Target

### 1. Any system event is subscribable (open vocabulary)

Every `LoomEvent` / `RigLogEvent` the system writes is also **dispatched**
as a system event to every subscribed consumer. The event ID is the
**variant name** (`KnotFailed`, `KnotCompleted`, `TimeoutExceeded`,
`QueueIdle`, `AgentInactivity`, `SessionResumed`, `KnotEmptyResponse`,
`ContextCompacted`, `StrandIgnored`, `StrandSkipped`, `LoomStarted`,
`LoomStopped`, `KnotRegistered`, `KnotDeregistered`, `KnotParseWarning`,
`DirectoryCreated`, `StrandProcessed`, `KnotProcessing`, `KnotEventsMissing`).
New log-event variants become subscribable automatically — no registry, no
allow-list, no restriction (explicit design decision: *allow all for now*).

Single exception (design decision, see Notes): **`EventsDispatched` is not
dispatched** — it is the dispatch record itself; dispatching it would be
self-referential (an `EventsDispatched` emission would need to record
itself). It remains observable in the loom-log.

### 2. Wildcard producer

`strand-dir` accepts `*` in the producer position:

```yaml
strand-dir: event:*:AgentInactivity      # any producer: any knot, any loom, the rig
strand-dir: event:writer:KnotFailed      # knot-scoped (existing)
strand-dir: event:writing-loom:KnotFailed  # loom-scoped (existing, -loom suffix)
strand-dir: event:dev-rig:QueueIdle      # rig-scoped (new)
```

Event ID remains **exact** (no `event:*:*` in this plan — noted as future
work).

### 3. Producer positions and their scopes

| Producer token | Matches |
|---|---|
| `*` | Any scope: knot-scoped, loom-scoped, and rig-scoped dispatches |
| knot id | Knot-scoped dispatches (existing 058 semantics: exact producer, or any known knot id) |
| `<loom-id>` (ends `-loom`) | Loom-scoped dispatches from that loom, and knot-scoped dispatches from knots in that loom (existing) |
| rig id (rig dir basename, e.g. `dev-rig`, `rig`) | Rig-scoped dispatches only (new) |

Dispatch scope per event:

- **Knot-scoped** (producer = the knot that ran; its loom is the loom-level
  match): all run-lifecycle and failure events — `KnotProcessing`,
  `KnotCompleted`, `KnotFailed`, `StrandProcessed`, `StrandIgnored`,
  `StrandSkipped`, `KnotEventsMissing`, `SessionResumed`,
  `KnotEmptyResponse`, `AgentInactivity`, `ContextCompacted`,
  `TimeoutExceeded`, `KnotRegistered`, `KnotDeregistered`,
  `DirectoryCreated`.
- **Loom-scoped** (producer = the loom, no knot): `LoomStarted`,
  `LoomStopped`, `KnotParseWarning` (knot file level only).
- **Rig-scoped** (producer = the rig): `QueueIdle`.

### 4. Dispatch mechanics — reuse, don't rebuild

System events are synthesized as `AgentEvent` structs
(`occurred: true`, `payload` map, `body`) and pushed through the **existing**
`EventDispatcherPort::dispatch` — identical event-file format, identical
per-consumer placement (`tie-offs/<rig>/<consumer-loom>/<EventId>/event-*.md`),
identical filename contract (070), identical watch/debounce/queue pipeline.
A consumer knot triggered by a system event reads the event file as its
strand, exactly as with agent events. No new watch, port, or file shape.

### 5. System event payloads

`payload` keys (set where applicable; absent keys are omitted):

| Key | Set by |
|---|---|
| `error` | `KnotFailed`, `StrandProcessed`, `TimeoutExceeded` (PortError / outcome `Display`) |
| `strand-path` | all run-scoped events |
| `session-id` | `SessionResumed`, `KnotEmptyResponse`, `AgentInactivity`, `ContextCompacted`, `KnotFailed`, `TimeoutExceeded` (when captured) |
| `blocked-call`, `silent-secs`, `window-secs` | `AgentInactivity` |
| `attempt` | `SessionResumed`, `KnotEmptyResponse`, `AgentInactivity`, `ContextCompacted` |
| `reason`, `tokens-before` | `ContextCompacted` |
| `reason` | `StrandIgnored`, `StrandSkipped` |
| `expected-events` (joined) | `KnotEventsMissing` |
| `knot-file-name`, `message` | `KnotParseWarning` |
| `directory` | `DirectoryCreated` |
| `tie-off-path` | `KnotCompleted` |

`body` = one human-readable summary line (the same text the loom-log entry
carries), so the consumer agent gets narrative context, not just keys.

### 6. Loop safety: self-exclusion (design decision)

A **system** event is never dispatched to the knot that produced it — the
producer knot is excluded from the consumer scan regardless of subscription
form (`event:*:KnotFailed` on knot K does not re-trigger K on its own
failure). Rationale: a terminal system event is by definition terminal;
transient retries are already owned by the internal nudge/resume loop
(078/079/081), and without exclusion a failing self-subscribed knot is an
unbounded run loop. Agent-emitted events keep today's semantics
(self-dispatch allowed there — an agent deliberately re-triggering itself).
Cross-knot cycles (A-fails→B, B-fails→A) are a knot-design responsibility:
failure subscribers should form an acyclic fan-out to sink knots (documented
in `knot-design`, no code guard).

## Non-Goals

- No event-ID wildcard (`event:*:*`) — future work.
- No payload-based filtering at subscription time (e.g. "only
  `KnotFailed` with a timeout") — consumers read the event file and decide.
- No new network listener — the filesystem interface is unchanged.
- No change to agent-emitted event semantics (tie-off blocks, `occurred`,
  enforcement follow-up, 059).
- No change to replay semantics — touching a system event file re-triggers
  the consumer, same as today.
- No new loom-log / rig-log entries beyond the existing
  `EventsDispatched` record (knot-scoped dispatches).

## Design

### Domain (`src/domain/`)

**`StrandSource` (value_objects.rs)**

- `from_str` already accepts any non-empty producer token — `*` parses with
  no change; add a test pinning `event:*:X`.
- `EventSubscription` gains two variants:
  ```rust
  Wildcard { event_id: String },
  RigLevel { event_id: String },
  ```
- `resolve_for_producer(producer_knot_id, producer_loom_id, all_knot_ids)`
  (knot-scoped dispatches) gains a wildcard branch checked **first**:
  `target == "*"` → `Some(Wildcard { event_id })`. Existing precedence
  (loom-suffix, then knot per 058) is untouched.
- New `resolve_loom_event(loom_id: &str) -> Option<EventSubscription>` for
  loom-scoped dispatches: `*` → `Wildcard`; `target == loom_id` (ends
  `-loom`) → `LoomLevel`; else `None`.
- New `resolve_rig_event(rig_id: &str) -> Option<EventSubscription>` for
  rig-scoped dispatches: `*` → `Wildcard`; `target == rig_id` → `RigLevel`;
  else `None`.
- `matches_event` style filtering (event-ID equality) is extended to the two
  new variants in the dispatch pass.

Rig id = the rig directory's file name (`rig_dir.file_name()`), the same
value the runtime root is derived from. Collision note (documented, not
guarded): a knot named `dev-rig` inside rig `dev-rig` is harmless — its
events are knot-scoped, and `event:dev-rig:QueueIdle` resolves only as
rig-level (rig-scoped dispatches never consult knot ids).

### Application (`src/application/`)

**`SystemEventEmitter`** (new service, `usecases/system_event_emitter.rs`):

```rust
pub enum EventScope {
    Knot { loom_id: LoomId, knot_id: KnotId, strand_path: Option<StrandPath> },
    Loom { loom_id: LoomId },
    Rig,                      // rig id taken from the emitter's rig_dir
}

impl SystemEventEmitter {
    pub fn emit(&self, scope: &EventScope, event_id: &str,
                payload: HashMap<String, String>, body: Option<String>)
        -> Result<Vec<Dispatch>, PortError>   // Dispatch = (event_id, consumer_knot, consumer_loom, path)
}
```

- Builds the `AgentEvent`, scans `store.list()` for consumers whose
  `strand_source` is an `EventUri`, resolves per scope (the three
  `resolve_*` functions), excludes the producing knot (knot scope), groups
  by `(consumer_loom, event_id)` with per-group sequencing (070), and calls
  `EventDispatcherPort::dispatch` per consumer.
- The grouping/sequencing block currently inline in
  `ProcessStrand::dispatch_events_to_consumers` is **extracted** into a
  shared helper used by both agent-event and system-event dispatch, so
  filename/seq behaviour is identical by construction.
- Injection: built in the composition root (`server.rs`), stored on
  `AppContext`; threaded into `ProcessStrand`, `session_resume::execute_with_resume`
  (new parameter), `ConfigEventHandler`, and the pipeline loop (for
  `QueueIdle`).

**Emission sites** — one `emit` call next to each existing log write:

| Event (scope) | Site |
|---|---|
| `KnotProcessing` (knot) | `process_strand.rs` `execute_inner`, after strand validation |
| `StrandIgnored` / `StrandSkipped` (knot) | `validate_strand` skip branches |
| `KnotFailed` (knot) | `execute_inner` config-error branch (ProfileNotFound / ModelRefNotFound) **and** `handle_failure` (outcome error) |
| `StrandProcessed` (knot) | `handle_success` (no error) and `handle_failure` (with error) |
| `KnotCompleted` (knot) | `handle_success` |
| `KnotEventsMissing` (knot) | event-enforcement block, both log sites |
| `TimeoutExceeded` (knot) | `execute_inner` timeout branch, next to the existing rig-log write |
| `SessionResumed` (knot) | `session_resume.rs` resume path |
| `KnotEmptyResponse` (knot) | both nudge-loop sites |
| `AgentInactivity` (knot) | both stall sites |
| `ContextCompacted` (knot) | compaction observation site |
| `KnotRegistered` / `KnotDeregistered` (knot) | `config_event_handler.rs` + `loom/register.rs` + `loom/unregister.rs` |
| `LoomStarted` / `LoomStopped` (loom) | `register_loom` / discovery; shutdown loop in `server.rs` |
| `KnotParseWarning` (loom) | warning loop in `register_loom` / `discover` |
| `DirectoryCreated` (knot) | `mod_watchers.rs` auto-create branch |
| `QueueIdle` (rig) | pipeline burst-end branch in `server.rs` (no `EventsDispatched` loom record — rig scope has no loom; console echo only) |
| ~~`EventsDispatched`~~ | **not emitted** (see Target §1) |

Dispatch failures stay non-fatal (best-effort, matching today's agent-event
dispatch): `emit` returns `Err`/partial and the site logs via `eprintln!`
(console) — the run outcome is unaffected.

### Adapters

`FileSystemEventDispatcher::dispatch` and `build_event_file_content` are
reused unchanged; the `producer` frontmatter value is the knot id, loom id,
or rig id per scope. No adapter changes beyond possibly re-exporting the
grouping helper.

## Phases

### Phase 0: Failing tests

Unit (`src/domain/value_objects.rs` tests):

- `event:*:X` parses; `*` + empty event id rejected as today.
- `resolve_for_producer`: `*` → `Wildcard` (any producer context, incl.
  loom-suffix producers); wildcard checked before loom/knot branches.
- `resolve_loom_event`: `*` → `Wildcard`, exact loom → `LoomLevel`, other
  knot/loom/rig id → `None`.
- `resolve_rig_event`: `*` → `Wildcard`, exact rig id → `RigLevel`, else
  `None`.

Emitter unit (`MockEventDispatcher` capturing dispatches, in-memory store
with EventUri consumers — pattern from `tests/event_fanout.rs`):

- Knot-scoped emit reaches knot-level, loom-level, and wildcard consumers;
  non-matching event ids and producer tokens do not.
- **Self-exclusion**: producer knot excluded even for `event:*:` and
  explicit `event:<self>:X`.
- Loom-scoped and rig-scoped emit reach loom-level/wildcard and
  rig-level/wildcard consumers respectively.
- Grouping/seq: same-directory fan-out gets `seq ≥ 1` names, singleton gets
  plain name (070 contract preserved for system events).
- `EventsDispatched` never emitted (emitter has no path for it).

Integration (`tests/`, mock-CLI harness, no live rig runs in this repo per
AGENTS.md):

- Producer knot run **fails** (mock CLI returns non-zero) with consumer knot
  `strand-dir: event:*:KnotFailed` → consumer's dispatch dir receives
  `KnotFailed/event-*.md` with `producer:`, `error`, `strand-path`
  frontmatter; consumer is enqueued and runs (mock CLI success) → consumer
  tie-off `Produced`.
- Timeout path (mock CLI hangs past a short timeout) with
  `event:<producer>:TimeoutExceeded` consumer → file delivered, no failed
  tie-off for the producer (077/081 outcome preserved), rig-log
  `TimeoutExceeded` still written.
- `QueueIdle`: after a burst with a wildcard `QueueIdle` consumer → event
  file delivered post-burst.

### Phase 1: Domain

`StrandSource`/`EventSubscription`/resolvers per Design. Existing 056/058
tests stay green (precedence unchanged; wildcard is additive).

### Phase 2: Emitter + shared grouping helper

Extract grouping from `dispatch_events_to_consumers`; implement
`SystemEventEmitter`; wire into `AppContext`. Agent-event dispatch behaviour
must be byte-identical (existing `tests/event_fanout.rs`,
`tests/event_enforcement.rs` green unchanged).

### Phase 3: Runtime emission sites

`process_strand.rs` + `session_resume.rs` sites (the "needed" set: run
outcomes and all failure classes).

### Phase 4: Lifecycle + rig emission sites

Config pipeline, loom use cases, `mod_watchers`, shutdown loop, and
`QueueIdle` in the pipeline loop.

### Phase 5: Integration + regression

Phase-0 integration tests green; full `cargo test`, `cargo clippy`;
mock-CLI harness end-to-end per Phase 0 list.

### Phase 6: Verify + docs + version

- `knot-create` skill: subscription grammar (wildcard + rig id), system
  event ID table (the full LoomEvent/RigLogEvent catalog with scopes and
  payload keys), payload reference.
- `knot-design` skill: loop discipline for failure subscribers (acyclic
  fan-out; self-exclusion rule; per-attempt events fan out per attempt —
  consumers must be idempotent).
- `knot-update` skill: changelog entry for **0.41.0** — new subscribable
  system events, `event:*:` wildcard, rig-scoped `QueueIdle`, self-exclusion
  rule; no document migration required (existing looms unaffected).
- `README.md` / user docs: one section — "reacting to knot outcomes".
- Version bump to 0.41.0; `cargo install --path .` after
  project-plan-completion; deploy skills per AGENTS.md (copy + diff
  verify).

## Test Strategy

- **Unit**: resolvers (domain, pure), emitter against mock dispatcher +
  in-memory store (application).
- **Integration**: mock-CLI harness (`MockAgentRunner` / mock `pi` script) —
  failure-class → event-file delivery → consumer run; 077/081 outcome
  contracts asserted alongside (no failed tie-off on timeout/inactivity;
  rig-log entry present).
- **Regression**: 056/058/059/070 suites untouched and green — wildcard is
  additive, grouping is extracted not rewritten, agent-event path unchanged.
- No live rig runs in this repository (AGENTS.md) — verification is
  `cargo test` / `cargo clippy` + the mock-CLI harness.

## Notes — design rationale

- **Open vocabulary over a curated list.** The exception catalog (Plan 077,
  079, 080, 081 outcome + `PortError` classes) maps 1:1 onto log-event
  variant names, which already carry the right context in the loom/rig
  logs. Making "every log event dispatchable" instead of a fixed 8-event
  set means the vocabulary can never lag the code: a new `LoomEvent`
  variant is subscribable the day it lands. Cost: per-attempt events
  (`SessionResumed`, `KnotEmptyResponse`, `AgentInactivity`,
  `ContextCompacted`) fan out once *per attempt*, not per run — a retried
  run can fire a consumer 2–4×. Accepted by design decision (allow all for
  now); the mitigation is the standard knot idempotency discipline,
  documented rather than enforced.
- **Wildcard over producer lists.** `event:*:KnotFailed` expresses "any
  knot that fails" without a producer allow-list that breaks when looms are
  added; the three existing positions (knot, loom, `*`) cover the
  specificity gradient, with rig id added for `QueueIdle`.
- **Self-exclusion only for system events.** Agent events are the agent's
  own choice (deliberate self-retrigger is a valid workflow); system events
  are terminal facts about the run, and the only sane "self" consumer of
  one's own terminal failure would be an infinite loop. Cross-knot cycles
  are left to design discipline: Knot has one loop-breaker (consume-on-
  failure) and knot idempotency, not a cycle detector.
- **`EventsDispatched` excluded.** It records the act of dispatching;
  dispatching it to subscribers would require recording its own dispatch —
  the only fixed point is "don't emit it". Subscribers who need
  "a dispatch happened" can watch the loom-log (out of scope for strand
  triggers).
- **`TimeoutExceeded` is knot-scoped, not rig-scoped.** Despite living in
  the rig-log, it carries `(loom_id, knot_id, strand_path)` — subscribers
  want "this knot timed out", and knot/loom/wildcard positions all work
  naturally. Only `QueueIdle` is genuinely rig-scoped.
- **Emission is best-effort and non-fatal.** A dispatch failure must never
  change a run's outcome or block the pipeline (parity with agent-event
  dispatch); failures surface on the console, per the existing error-
  surface contract.

## Implementation Status: ⬜ Not started
