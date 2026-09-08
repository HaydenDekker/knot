# Plan 086: Graceful Task Handoff — Checkpointed Continuation Chains for Task-Bearing Sessions

## Related Plans

Builds on [084 Graceful Completion](../084-graceful-completion/graceful-completion-plan.md)
(the `pi-rpc` runner, `get_session_stats` context sampling, and the
`steer` mechanism — this plan **reuses the 084 steer channel verbatim**
and only changes the *payload* it sends at the water-mark: a handoff
instruction (`HANDOFF_NOTE`) instead of the terminal `WRAP_UP_STEER`
wind-down — for **every** knot on a water-marked alias; the same
`HANDOFF_NOTE` is also the injected prompt for the new `pi-json`
water-mark stop-resume),
[079 Context Overflow — Compact and Continue](../079-context-overflow-compact-and-continue/context-overflow-compact-and-continue-plan.md)
(compaction records; the *reactive lossy* half that graceful handoff
supersedes for task work),
[080 Overflow Fail Fast](../080-overflow-error-fail-fast/overflow-error-fail-fast-plan.md)
(`ContextLimitReached` terminal classification — stays the last-resort
safety net, "Tier 2" in this plan's two-tier model),
[081 Inactivity Timeout](../081-inactivity-timeout/inactivity-timeout-plan.md)
(the watchdog the handoff chain inherits; its budget arithmetic is the
same global-deadline discipline this plan extends across handoffs),
[078 Final-Response Request](../078-final-response-request/final-response-request-plan.md)
(the greppable-const convention for agent-facing nudge text — this plan
adds `HANDOFF_NOTE`),
[082 System Event Subscriptions](../082-system-event-subscriptions/system-event-subscriptions-plan.md)
(the `TasksIncomplete` / `BatchIncomplete` system events + service-log
lines), and
the **late-removal / at-least-once queue** model (see
`execute_with_pending`
and `master-plan` row for the `knot step` / late-removal plan) whose
idempotency guarantee makes the continuation chain safe.

It also rides on two **existing Knot mechanisms**, unchanged in
behaviour:

- **The tie-off event rule** — the `# Subscriber Events` prompt block
  (`src/domain/events.rs`): every knot's prompt lists its declared
  subscriber events, and the agent **must acknowledge each one in its
  tie-off** with one ```markdown block carrying `event:` and a
  **required explicit `occurred: true | false`** (+ `description`, and
  `timestamp` + event-specified optional fields when `true`).
  `occurred: false` is not dispatched but counts as acknowledgement; a
  missing block is caught by the existing `KnotEventsMissing` loom
  event.
- **The 084 water-mark steer** — `ContextWrapUpSteered`, recorded
  fire-once per invocation when the water-mark payload is sent.

## Problem

Overflow is the common denominator of every "one knot does a lot of
work" shape. Whether a knot **accumulates** a list of tasks in one
session, or **scopes-then-fans-out** into per-task sessions, the thing
that breaks it is the context window. The previous generation of this
rig (the CI×UAT validation loops) showed the failure precisely: a
session that understood a large scope (test plan, VCRM, build identity,
the test run) then worked through several similar combos accumulated
context until it **overflowed**, and an overflow ends with *no tie-off,
no commit, no notes* (the 079/080 story). [Plan 084](../084-graceful-completion/graceful-completion-plan.md)
softened this (steer the session to wrap up before the limit), but the
wrap-up is a *terminal wind-down* — it commits and stops; it does not
*continue* the work in a fresh, bounded session.

Two facts shape the fix:

1. **The agent cannot see its own context usage.** Token usage is
   written to the session *log* (`usage` on each assistant message) and
   shown to the *human* (`/session`), never injected into the prompt.
   So a knot can never *self-gate on context*. The only reliable
   context trigger is Knot-side — `get_session_stats` (`pi-rpc`) or
   per-message stream `usage` (`pi-json`) — and the only pre-overflow
   *injections* are the `steer` channel (`pi-rpc`) and the stop-resume
   prompt injection (`pi-json`, added by this plan).

2. **A handoff spawns a new session, and a new session gets a fresh
   timer.** [session_resume::execute_with_resume](../../../src/application/session_resume.rs)
   captures `start = Instant::now()` once per invocation and bounds its
   retry loop by `profile_timeout` vs `start.elapsed()` (up to
   `MAX_RETRIES = 10`, bailing under `MIN_REMAINING_SECS = 5`). That
   budget is **global within one invocation** — but a handoff is a
   *second* invocation, and today it would resolve a fresh
   `profile_timeout = profile.session_timeout()` from a fresh `start`.
   Left as-is, a chain of handoffs would reset the wall-clock clock on
   every hop and could run forever.

**What we want.** A knot that can work through a **checklist** across
one or more sessions, **handing off gracefully** when its context
water-mark is crossed and **resuming in a fresh, bounded session** from
the durable checklist — so overflow is *avoided* where the adapter can
observe context (`pi-rpc`, `pi-json`; not just recovered from), the
continuation is **bounded** (fresh session, not the accumulated
history), the work is **recoverable** on every adapter, and the **whole
handoff chain obeys the original timer** (a handoff buys a fresh
*context*, never a fresh *budget*).

This is bigger than "batched events." The tasks may be similar (the
CI×UAT combos) or unrelated (a list of review items) — the mechanic
hangs off the **checklist**, not off the tasks being alike.

## The model

**One axis, one denominator.** "Single session" (accumulate) and
"multi session" (scope-then-fork) are two policies over *how much work
goes into a session*. Overflow is what breaks both. The graceful
**handoff** is the overflow manager that makes either policy safe.

**Two tiers, degrading but always recoverable.**

| Tier | Trigger | Author of the handoff | Adapters | Gracefulness |
|---|---|---|---|---|
| **1 — context water-mark** | context usage crosses `ctx-wrap-up-limit` | agent — steered by Knot (`pi-rpc` steer) or stop-resumed by Knot (`pi-json`) | **pi-rpc + pi-json** | graceful, context-accurate, pre-overflow |
| **2 — overflow / failure** | the session actually overflows or fails | the rig — the re-entered knot resumes from the checklist | **all** | recover, not graceful |

Tier 2 **already exists** (080's `ContextLimitReached` / 081's timeout
terminal handling writes a terminal tie-off and consumes the event; the
next work is a fresh dispatch). This plan adds Tier 1 — the 084 steer
re-pointed at a handoff on `pi-rpc`, plus the `pi-json` water-mark
stop-resume — and the **continuation chain** they drive.

**The keystone is a durable, mutable checklist — owned by the rig, not
by Knot.** Everything else hangs off it: it is what makes the handoff
*progress-carrying* (done / in-progress / pending per item), what makes
the continuation *bounded* (a fresh session reads the checklist, not
the accumulated history), and what makes every tier *recoverable* (a
re-dispatch resumes the in-progress item). Its **format, location, and
lifecycle are the rig designer's**: the first session of a task-bearing
knot can author it, or an upstream knot can — e.g. a *planner* knot
writes a phase checklist that a *phase-runner* knot works from. Knot
never reads, parses, or writes the checklist; the seam is the
**`TasksIncomplete` declaration in the tie-off** — a session asked to
wrap up at the water-mark declares `occurred: true` (continue) or
`occurred: false` (complete); a session that finished without the note
simply completes. Knot sees the declaration, never the checklist
content. Handoff context is a
**pointer to durable state**
(the checklist + committed files) plus **two carried context fields**
(the agent's `background-additional`, accumulated by Knot across the
chain, and its one-shot `next-task-context`) — never a re-statement of
the context. That is what keeps it small *and* robust, and is the
difference between handoff and lossy compaction.

## Target — behavioural contract

### Configuration — default-on, no new keys

**No new front-matter, no new `models.yml` key, no per-knot config.**
The handoff contract is **enabled by default for every knot** — a
session either finishes or gets the wrap-up ask at the water-mark, and
a session *asked to wrap up* must **declare the outcome explicitly**
in its tie-off:

- `TasksIncomplete` is an **implicit *self-consumption* for every
  knot** (the knot is its own consumer; see **Self-continuation
  dispatch**) — but the prompt entry and the acknowledgement
  obligation are **scoped to the water-mark**:
  - The knot's `# Subscriber Events` prompt block lists the self
    `TasksIncomplete` entry **only when the knot's profile alias
    carries `ctx-wrap-up-limit`** — the note can fire only there, so
    the format must be known before it can. On a non-water-marked
    alias the block is unchanged and the session is exactly today's.
  - A session **that received the note** (water-mark steer or
    stop-resume) **must acknowledge it explicitly in its tie-off —
    `occurred: true` (work remains → self-continuation) or
    `occurred: false` (nothing remains → explicit batch completion)**,
    per the existing tie-off event rule. A missing acknowledgement on
    such a session is caught by the existing `KnotEventsMissing`
    enforcement (the expected-events set includes the self entry only
    for water-marked sessions). This makes "batch complete" a
    *positive declaration* in exactly the tie-off where the wrap-up
    was asked — never an inference from absence.
  - A session that **finished without the note** carries no
    `TasksIncomplete` block — plain `KnotCompleted`, as today.
- `ctx_wrap_up_limit` (model alias, from 084) is **reused as the
  water-mark** — the Tier-1 threshold, now consumed by *both* the
  `pi-rpc` steer monitor and the `pi-json` stop-resume monitor. No new
  `models.yml` key in v1. **The water-mark is what makes Tier 1
  exist**: with no `ctx-wrap-up-limit` on the alias, *no* adapter has
  a pre-overflow handoff, the prompt carries no `TasksIncomplete`
  entry at all, and the session behaves exactly as today (overflow
  recovers via Tier 2). With one, the entry is in the prompt, the note
  can fire, and a note-driven stop must declare `occurred: true|false`
  (a voluntary stop with work remaining may declare `occurred: true` —
  the format is known — but a normal completion owes no block).
  Water-mark steering is already fleet-wide per alias (084); this plan
  changes the *payload* it sends (below).
- **The chain cap is a global constant**: `MAX_CONTINUATIONS = 10`
  (mirrors `MAX_RETRIES`). A per-knot `max-continuations` knob is
  deliberately out of v1 (see Non-Goals).
- **There is no task-count gate.** An earlier draft of this plan had a
  `tasks-per-session` (K) count gate as an all-adapter, agent-gateable
  first tier; it is **dropped**. It was a heuristic that bounded *count*
  not *size*, and it made every task knot maintain a counter the agent
  had to gate on — bookkeeping for a coarse bound. The water-mark covers
  the graceful case (now on two adapters); overflow recovery (Tier 2)
  covers the rest. `pi-stdio` — no `usage` in its plain-text stream —
  is the one adapter left at Tier 2 only (manual re-trigger per hop).
- **No declared self-subscription.** A knot has exactly one
  `strand_source` (its only input). The implicit, knot-scoped
  self-continuation at dispatch time (see **Self-continuation
  dispatch**) replaces it. A declared loom-level
  `event:<own-loom>:TasksIncomplete` subscription is rejected: it would
  crowd out the knot's work input (one `strand_source`) and cross-fire
  between knots in the same loom.

### Self-continuation dispatch (how the chain re-enters the knot)

A knot has **one declared input** (`strand_source`: a filesystem strand
dir *or* an event URI — never both). A task-bearing knot needs *two*:
the **work input** that starts a batch (producer strand files, manual
re-triggers) and the **self-continuation** that carries the chain. v1
resolution:

- The declared `strand_source` stays the **work input** (v1 scope:
  task-bearing knots are filesystem-strand-triggered).
- The self-continuation is granted **implicitly and universally at the
  dispatch level**, not the declaration level: in
  `dispatch_events_to_consumers`, a knot matches its *own*
  `TasksIncomplete` event when the tie-off declared `occurred: true` —
  producer == the knot's own id ∧ event id `TasksIncomplete` ∧
  `occurred: true`. This is the knot-scoped match an
  `event:<self>:TasksIncomplete` URI would express, granted implicitly
  instead of a second `strand_source` — for **every** knot, since the
  contract is default-on.
- The dispatched (stamped) event file is delivered **into the knot's
  existing input** — for a strand-dir knot, its own strand dir — and
  the existing watcher/queue picks it up like any strand-with-event-
  context. No new front-matter, no new subscription value.

Knot-scoped matching (producer == self) eliminates the cross-fire that
a loom-level subscription would have: two task-bearing knots in one
loom never receive each other's handoffs.

### State split (two owners, no two-writer file)

- **Agent-owned: the checklist.** Wherever the rig designer puts it (a
  `project/` document, a `tie-offs/…` record, …) and in whatever format
  serves the domain. The items, their statuses, per-item progress, and
  **state-pointers** (where the durable work lives). The agent
  writes/updates it as it works. Knot never reads or writes it.
- **Knot-owned: the stamps on the dispatched event file's
  front-matter** — `batch-deadline-epoch`, `continuations`, and the
  **accumulated `background-additional`** block. These are Knot's
  timer/loop bookkeeping, stamped onto the *dispatched* event file (the
  Knot-authored copy written for the consumer at dispatch time) — never
  into the agent's source event file, never into the checklist. The
  agent never touches the budget; the agent never sees a clock.

An example checklist (the shape is the *rig's*, not Knot's — shown
only to fix the handoff vocabulary):

```yaml
tasks:
  - id: S1-add
    title: "Add a todo — happy path"
    status: done            # pending | in-progress | done | failed | split
    spec: "Run e2e-app, map S1.*, append record, bump VCRM line"
    commit: a1b2c3d         # commit that did it ("" until done)
    state-pointers:         # the handoff points HERE, not at prose
      - project/vcrm.md
      - tie-offs/rig/.../records.md
  - id: S5-persist
    title: "Persistence reload"
    status: in-progress     # the session ran out mid-item
    progress: "ran e2e, mapped S5.persist; still: append record + bump VCRM"
```

### The handoff contract — where the agent learns it

The contract has three delivery points, and only the second and third
are Knot's:

**1. Rig-designer content (the knot's `prompt_template`).** Everything
about the checklist is the rig's to design:

- **Who authors it.** The first session of the knot, or an upstream
  knot — e.g. a *planner* knot implements a phase checklist and the
  *phase-runner* knot works from it.
- **Its format and location.** Whatever serves the domain (YAML list,
  markdown table, BDD spec, …).
- **Scope once.** Do the shared, expensive understanding (the CI×UAT
  case: test plan, VCRM, build identity, the test run) **once**, and
  record the durable results to files.
- **Work the checklist.** Process items in order; commit per item;
  update statuses as you go.
- **Split if too large.** *"If a single item is too large to finish in
  one session, break it into smaller items rather than attempting it
  whole."* This is the rig-side termination guarantee (see
  **Termination**).

**2. The `TasksIncomplete` entry in the `# Subscriber Events` block
(listed on water-marked aliases, via the existing tie-off event
rule).** The self entry's event description is the **handoff
contract** — the greppable const `TASKS_INCOMPLETE_DESCRIPTION`:

- *You self-consume this event. When a wrap-up note asks you to stop
  (steered or stop-resumed at the water-mark): first update your
  durable checklist/state and commit in-flight work, then acknowledge
  `TasksIncomplete` in your tie-off — `occurred: true` with a **body**
  that points at the checklist and the committed state (never a
  re-statement of the context), `next-task-context` — the operational
  brief that gets the next session up and running on the in-flight
  item, and `background-additional` — new persistent facts for the
  remainder of the chain (thin: facts and pointers, not narrative;
  anything that must outlive the batch also goes into the checklist)
  if work remains; `occurred: false` if nothing remains — the explicit
  "batch complete" declaration. If you stop with work remaining for
  any other reason, declare `occurred: true` the same way. If you
  finish your work normally without a wrap-up note, no block is
  needed. Declare `occurred: true` **only if work genuinely remains
  and your durable state points at it** — a spurious `true` costs a
  continuation hop.*

**3. `HANDOFF_NOTE` at the water-mark (the decision moment, Tier 1
only).** The 084 steer payload / `pi-json` stop-resume injection —
replacing the terminal `WRAP_UP_STEER` wind-down for every knot on a
water-marked alias:

- *You have crossed the context water-mark: wrap up now. Commit
  in-flight work and update your durable state, then acknowledge the
  `TasksIncomplete` event in your tie-off per its description —
  `occurred: true` with the handoff fields if work remains,
  `occurred: false` if nothing remains.*

On a water-marked alias the contract is thus **always in the prompt**
(the subscriber entry) so the *format* is never a surprise when the
note arrives; the **decision** — true or false — is made at the stop
the note triggers. On a non-water-marked alias neither the entry nor
the note exists, and the session is exactly today's session. Knot
stays **completely checklist-blind**: it never reads the checklist;
the tie-off's declaration is the seam.

### `TasksIncomplete` — an explicit tie-off declaration, true or false

The handoff follows the **existing tie-off event rule**, not a
side-channel. On a **water-marked session** — the one the note asked
to wrap up — the batch outcome is a positive declaration, never an
inference from absence:

- **`occurred: true`** — work remains. The block carries the handoff
  fields (below) and is **dispatched** through the existing
  `dispatch_agent_events` path, re-entering the same knot via its
  implicit self-continuation; the `TasksIncomplete` loom event is
  recorded. (Any session that knows the format — a water-marked
  alias's — may declare it voluntarily; `reason: voluntary`.)
- **`occurred: false`** — nothing remains. Only from a **water-marked
  session** (the note asked): not dispatched (the existing rule:
  `occurred: false` counts as acknowledgement only). The tie-off
  itself is the explicit "batch complete" declaration; the run is
  `KnotCompleted` with no continuation.
- **Missing block** — normal for a session that **finished without
  the note** (plain completion, as today). For a **water-marked**
  session it is the "handoff-missed" case: the existing
  `KnotEventsMissing` enforcement fires (the expected-events set
  includes the self entry only for water-marked sessions; see
  **Observability**).

The run is a **success either way** (`TieOffOutcome::Produced` — the
session completed cleanly, with a **full tie-off** as any successful
run writes). Reusing the event machinery means tie-off writing, git
versioning, late removal, and event enforcement are **untouched**. On
the water-mark steer (pi-rpc) or stop-resume (pi-json), the agent wraps
up and writes the full tie-off declaring `TasksIncomplete` `true` or
`false`; without the note, a finished session writes the plain
tie-off.

**Event fields** (the agent's tie-off block; the dispatched copy
carries these plus the Knot stamps):

`occurred: true` — the handoff:

```markdown
```markdown
---
event: TasksIncomplete
occurred: true
description: Work remains — S5-persist in progress, 4 items pending
timestamp: 2026-09-08T14:32:00
next-task-context: |        # one-shot — injected into the NEXT session only
  S5-persist 80% done: e2e ran, S5.persist mapped; the record append is
  written but not committed; the VCRM line still needs the sha.
background-additional: |    # accumulated — Knot carries this forward to ALL later sessions
  The e2e-app S3 view flaps on first run; use --retry-once. Build sha 9f2c
  is the verified identity for this phase.
tasks-done: 3               # OPTIONAL — visibility only, agent-populated,
tasks-remaining: 4          # Knot never validates; when present, shown in the
                            # loom event + service log as live progress
---

Batch handoff — checklist: project/phases/07-implementation/checklist.md
Setup is done and COMMITTED. Do NOT re-derive it.
Durable state (read these, do not recompute):
  - build verified: sha <x>; dist sha256 <y>
  - e2e-app results: tie-offs/.../records.md
Done: S1-add, S2-complete. In-progress: S5-persist (see checklist).
Pending: S4-remove, web-e2e-regression-gate.
Continue with S5-persist, then the pending items. Commit per item.
```
```

`occurred: false` — the explicit batch-complete declaration:

```markdown
```markdown
---
event: TasksIncomplete
occurred: false
description: Batch complete — all checklist items done and committed;
  nothing in flight
---
```
```

The handoff **points at** the checklist and the committed files; the
checklist + commits are the ground truth. A thin handoff is survivable
because the pointers resolve to durable artifacts — the anti-"re-
derive" property.

**The two context fields, and who carries what:**

- **`background-additional` — Knot accumulates.** When dispatching the
  continuation, Knot appends this event's contribution to the running
  total and stamps the *accumulated* block on the dispatched event's
  front-matter (hop-labelled contributions, emission order,
  **append-only** — never reordered or rewritten). Every later session
  in the chain gets the whole block. The accumulation is **chain-
  scoped**: it rides the event chain. If the chain stops (cap or
  deadline) and the batch spans to a later dispatch, the new dispatch
  starts with *no* accumulated background — facts that must survive a
  batch span belong in the **checklist** (durable, git-tracked, agent-
  owned). The layering rule: *durable state → checklist; must-know-
  without-reading facts → `background-additional` (and into the
  checklist if they must outlive the batch).*
- **`next-task-context` — one-shot.** Injected into the *next* session's
  prompt only; not carried forward. Each hop's agent writes its own for
  the hop after it.
- **Growth is bounded** by `MAX_CONTINUATIONS` (one blob per hop,
  ≤ 10); the contract keeps contributions thin.

### The continuation prompt (assembly order — caching matters)

When Knot processes a `TasksIncomplete` dispatch (a `occurred: true`
declaration), the fresh session's prompt is assembled in this **fixed
order**:

1. `listener_context` + `base_prompt` — the static knot instructions +
   persona (cheap, re-sent; identical every hop).
2. **Accumulated `background-additional`** — the Knot-stamped,
   append-only block (hop N's block is a byte-identical **prefix** of
   hop N+1's).
3. **Handoff block** — the event body (per-hop, agent-authored).
4. **`next-task-context`** — the one-shot brief (per-hop, last).

The *accumulated conversation* is what a fresh session drops; the
*instructions* are static and re-sent. Bounded + progress-carrying.

**Why this order (input caching).** LLM providers cache the *prefix* of
a prompt. The accumulated background is the one section that grows
monotonically, so it must sit *before* the per-hop variable sections
(handoff body, `next-task-context`) and be append-only/deterministic —
then each hop re-uses the cached prefix up to and including the
previous hops' background, and only the new tail (new contribution +
handoff + brief) is novel. If a per-hop variable section sat *before*
the background, every hop would bust the cache at the first variable
section and the entire accumulated block would be reprocessed cold.
Equivalently: a later task's `next-task-context` must never replace or
precede the previous tasks' accumulated background. (`resolve_config_
and_build` already prepends `listener_context` to `base_prompt`; the
three carried sections are appended in the order above.)

### The global timer (the core fix) — the original deadline is obeyed

**A handoff buys a fresh context, never a fresh budget.**

- The **first** invocation of a batch (the setup session) records
  `batch_deadline_epoch = now + profile.session_timeout()` (the same
  wall-clock budget a single session would have had) and stamps it,
  with `continuations: 1`, on the `TasksIncomplete` it dispatches.
- A **continuation** invocation resolves its timeout from the deadline,
  **not** from a fresh `profile.session_timeout()`:

  ```text
  // in resolve_config_and_build, when the event carries batch_deadline_epoch:
  profile_timeout = batch_deadline_epoch - now     // remaining GLOBAL budget
  ```

  Because `execute_with_resume` computes its per-attempt timeout as
  `profile_timeout - start.elapsed()` and bails under
  `MIN_REMAINING_SECS`, **passing the remaining global budget as
  `profile_timeout` makes the entire Tier-2 retry chain obey the
  original timer for free** — one change, everything downstream obeys.
  A per-session cap of `min(profile.session_timeout(), remaining)` may
  be layered on if a rig wants a single hop bounded below the batch
  deadline; v1 uses the remaining global budget directly.
- **The stamp is inherited verbatim.** When a continuation emits its
  own `TasksIncomplete`, Knot copies the incoming
  `batch-deadline-epoch` **unchanged** (and `continuations + 1`) onto
  the new event — the deadline is *never* recomputed from the current
  invocation. Recomputing "this invocation's start +
  `profile_timeout`" would re-extend the deadline by each hop's own
  startup/queue-wait drift — a silent per-hop budget reset that
  violates "never a fresh budget".
- If, when a continuation is dequeued, `batch_deadline_epoch - now <
  MIN_REMAINING_SECS`, Knot does **not** spawn a session. It writes a
  **Knot-authored degenerate terminal tie-off** (a deferral, not a
  failure: "batch deadline exhausted; remaining work is in the
  checklist; resume on the next dispatch"), records
  `BatchIncomplete` (reason `deadline`), and the event is
  **late-removed through the normal path** — removal is *caused by the
  tie-off*, so a path with no tie-off would leave the queue entry
  cycling at the head of the knot's queue forever. The chain ends for
  this dispatch; the next work is a **fresh dispatch** (new producer
  trigger or manual re-trigger) that starts a **new batch** — fresh
  budget, `continuations` reset — and resumes from the checklist
  (which works precisely because the checklist path is in the knot's
  static prompt, not in the event). The chain never tight-loops with no
  time left.

**The `profile_timeout` field is the single lever.** It is already
threaded `resolve_agent_config` → `resolve_config_and_build` →
`execute_with_resume`. The change is purely: *for a continuation
event, derive it from the batch deadline instead of the profile.*

### Max continuations — the count cap (separate from the timer)

Each hop is a spawn + a commit + spend. Even with time left, cap the
chain: the first dispatched event carries `continuations: 1`; each
subsequent event carries the incoming count + 1. When a continuation
with `continuations == MAX_CONTINUATIONS` (global constant, `10`,
mirroring `MAX_RETRIES`) declares `TasksIncomplete` `occurred: true`,
Knot **stops**: it suppresses the dispatch, records a
`BatchIncomplete` loom event (reason `caps`), and leaves the
checklist for a later dispatch. This bounds the session count
independent of wall-clock (protects against a chain making tiny
progress fast). The cap bounds the **automatic** (Tier-1) chain; see
**Termination** for what bounds Tier-2 chains.

### `BatchIncomplete` — what it means

The **"the batch stopped early and work remains"** signal — the answer
to "why did this knot stop when the checklist clearly isn't done".
Fires in exactly two cases:

- **`reason: deadline`** — a continuation was dequeued with no budget
  left: no session spawned, degenerate tie-off written, chain ends for
  this dispatch.
- **`reason: caps`** — `continuations == MAX_CONTINUATIONS` and the
  agent declared `occurred: true`: dispatch suppressed, chain ends.

In both cases the work is **not lost** (checklist + per-item commits
are durable) and the batch is **not failed** (it is *deferred*): it
resumes on the next dispatch (new producer trigger or manual re-
trigger) with a fresh budget and a reset chain count. That is what
distinguishes it from its neighbours: `KnotFailed` = a run failed;
`KnotCompleted` = a run finished (with a `TasksIncomplete`
`occurred: false` declaration = batch *complete*); `BatchIncomplete` =
chain paused with work remaining. Exists as loom event + 082 system
event + greppable service-log line so the pause is observable and
subscribable.

### Termination

The **automatic** (Tier-1) continuation chain always terminates:

- **Progress is monotonic (rig-side).** Each session either completes
  ≥ 1 checklist item (the checklist shrinks toward empty) or **splits**
  an in-progress item into strictly smaller sub-items. The split rule
  is rig-design content (Knot cannot verify checklist content — see
  **Notes**); items are bounded below (an item is at least one
  indivisible action — it cannot be split forever), so eventually an
  item is small enough to complete.
- **Empty checklist ⇒ the chain ends.** If the final hop was
  water-marked its tie-off declares `occurred: false` (the explicit
  batch complete); if it finished before the note, it is a plain
  `KnotCompleted` — no continuation either way.
- **Hard caps otherwise.** `MAX_CONTINUATIONS` or the global deadline
  stops the chain with `BatchIncomplete`; the remainder re-queues for a
  later dispatch. Because the caps fire, a batch that legitimately
  needs more than one budget simply **spans multiple dispatches** — the
  checklist carries it, with no re-work.

**Tier-2 chains are bounded differently.** Overflow hops (a `pi-stdio`
knot, or a water-mark wrap that itself overflows) end in terminal
failures that consume their event; each recovery is a *fresh dispatch*
— a new batch with a reset `continuations` and a fresh budget. Nothing
carries chain state across dispatches, so a Tier-2 chain's total hop
count is **not** bounded by `MAX_CONTINUATIONS`; its termination rests
on the split rule (rig design) and operator re-triggering. `pi-rpc`
and `pi-json` make the automatic Tier-1 chain the norm; **`pi-stdio`
is the manual-recovery adapter** (manual re-trigger per hop).

So the "single item larger than the window" case is not a failure — it
becomes more items (the split rule), and the caps bound how long the
automatic chain takes.

### Adapter ladder (gracefulness varies; recovery is universal)

| Adapter | Tier 1 (water-mark handoff) | Tier 1 mechanism | Tier 2 (recovery) |
|---|---|---|---|
| **pi-rpc** | ✅ | live `steer` with `HANDOFF_NOTE` at `ctx-wrap-up-limit` | checklist resume (re-dispatch) |
| **pi-json** | ✅ | **stop-resume**: SIGINT at `ctx-wrap-up-limit`, re-invoke with `--session-id` + injected `HANDOFF_NOTE` | checklist resume |
| **pi-stdio** | ✗ (no `usage` in the plain-text stream) | — | checklist resume (manual re-trigger per hop) |

**The `pi-json` stop-resume mechanism.** All three pieces already
exist; the plan wires them together:

- The `pi-json` runner already parses **per-message `usage`**
  (input/output/cache/total) and the **`session_id`** from the JSON
  stream. A monitor applies the 084 fire-once / `ctx-wrap-up-limit` /
  null-skip discipline to stream `usage`.
- On crossing: **SIGINT** the process (bounded teardown grace, force-
  kill beyond — the pi-rpc teardown discipline), and the invocation
  ends as a **retryable "water-mark stop"** outcome carrying the known
  session id (distinct from a hard failure — the session file is
  durable and resumable).
- The **existing session-resume retry** (the `--session-id` re-
  invocation path) re-invokes with the **injected `HANDOFF_NOTE`
  instruction** (the same text the `pi-rpc` steer sends — one
  instruction, two delivery mechanisms). The injection records
  `LoomEvent::ContextWrapUpSteered` (the additive
  `mechanism: stop-resume` field; 084's pi-rpc path records
  `steer` by default). The
  resumed session still has the full context; it converges in the
  remaining headroom (`contextWindow − reserve − limit`): commit,
  update the checklist, write the full tie-off declaring
  `TasksIncomplete` with the two context fields (or `occurred: false`
  if nothing remains).
- **Within one invocation** (the same `start`), so the retry loop's
  existing `profile_timeout − start.elapsed()` arithmetic means the
  global clock is untouched — no deadline change needed for the
  wrap-up.
- **Loss mode:** if the resumed wrap-up *itself* overflows (the
  water-mark leaves headroom, so this is pathological), it is terminal
  Tier 2, as today.
- `HANDOFF_NOTE` is **generic** — Knot cannot name the checklist path
  (checklist-blind); it points at the `TasksIncomplete` event
  description already in the prompt.

The **durable checklist + per-item commits + idempotent knots** give
every adapter a *bounded, recoverable* batch. **`pi-rpc` and
`pi-json` get the graceful, context-accurate pre-overflow handoff**
(steer / stop-resume respectively). The Tier-1 intervention is
necessary precisely because the agent cannot self-detect context.

### Observability

**No new injection-moment event and no new `state.json` keys in v1.**
The chain lifecycle is observable at each hand-off point using the
existing events plus two new loom events and greppable
`[task-loop] …` service-log lines:

- **Injection moment** (Knot → agent, water-mark): 084's
  `ContextWrapUpSteered` — now universal across both Tier-1 adapters.
  Fields as 084 (`context_tokens`, `limit`, `attempt`, `session_id`)
  plus the **additive** `mechanism` field (`steer | stop-resume`,
  serde-defaulted to `steer` — 084's pi-rpc records are unchanged).
  Fire-once per invocation (the 084 discipline). This is the line that
  says "the handoff note went in."
- **Response moment** (agent → Knot, tie-off returned): the existing
  `KnotCompleted` / `KnotFailed` tie-off event (unchanged) + the
  parsed declaration:
  - `occurred: true` → the new **`LoomEvent::TasksIncomplete`**
    (recorded when the response is processed, alongside the
    `KnotCompleted` tie-off event). Fields `loom_id, knot_id,
    strand_path, session_id, continuations, tasks_done,
    tasks_remaining, deadline_epoch, reason, timestamp`.
    `reason ∈ {water-mark, voluntary}` (`water-mark` = a
    `ContextWrapUpSteered` fired this session; `voluntary` = the agent
    declared `occurred: true` at a stop with no water-mark) — the
    field ties the returned response back to the injection. Plus the
    self-dispatch and `[task-loop] handoff (knot=…, continuations=…,
    remaining=…, trigger=water-mark|voluntary)`.
  - `occurred: false` → nothing new: the tie-off itself is the
    explicit "batch complete" declaration (no dispatch, no loom
    event). Only a water-marked session carries it — the note asked.
  - Missing block → normal on a **non-water-marked** completion. On a
    **water-marked** session the existing **`LoomEvent::
    KnotEventsMissing`** fires (expected events include the self
    `TasksIncomplete` only then) and the correlation is logged:
    `[task-loop] handoff-missed (knot=…, session=…)` — the gap is
    explicit, not merely inferable.
- **Continuation moment** (Knot → fresh session for the same task): the
  self-dispatch is recorded by the existing `EventsDispatched` event
  (consumer = the knot itself), and the new session's start — the
  existing `KnotProcessing` event — is accompanied by `[task-loop]
  continuation (knot=…, hop=N/10, deadline=…, remaining=…s)` once the
  deadline-derived timeout is resolved, so the fresh session is
  correlated to the batch it continues (fresh context, never a fresh
  budget).
- **`BatchIncomplete`** — the chain-stopped signal (fields as above,
  `reason ∈ {deadline, caps}`), also as the greppable `[task-loop]
  batch-incomplete (knot=…, reason=…, continuations=…)` service-log
  line.
- `tasks_done` / `tasks_remaining` are **optional, agent-populated,
  visibility-only** — Knot never validates them; when present they make
  mid-chain progress visible in the loom event + service log.
- New **system events** (082) `TasksIncomplete` / `BatchIncomplete` so
  strands can subscribe off them.
- **Missed-handoff pattern:** `ContextWrapUpSteered` + a tie-off
  missing the `TasksIncomplete` block → `KnotEventsMissing` (with
  `TasksIncomplete` in `expected_events`) + the `[task-loop]
  handoff-missed` line = the note asked for a declaration but the
  agent never gave one → the batch is stalled for this dispatch but
  resumable on the next one; check the checklist. Documented in
  troubleshooting. Note `occurred: false` is **not** a miss — it is
  the agent explicitly saying the batch is complete.
- Success story, one hop: `KnotProcessing → ContextWrapUpSteered →
  KnotCompleted (+ TasksIncomplete, occurred: true) → EventsDispatched`
  (the self-continuation)`→ KnotProcessing` (continuation, hop N+1)`→
  KnotCompleted` (final hop — `occurred: false` if the note fired,
  plain completion if it finished first) `→ StrandProcessed`;
  one tie-off + one commit per hop; each continuation prompt carries
  the accumulated background (cached prefix) + handoff + one-shot
  brief.

## Non-Goals

- **Session forking (`pi --fork` / `createBranchedSession`).** Forking
  is the *pre-emptive, lossless* special case (the handoff is "the full
  checkpoint transcript"). This plan ships the *bounded, targeted*
  handoff (a fresh session + a pointer block) as the single mechanism;
  forking stays a possible later optimization, not v1.
- **A batch budget larger than one session's `timeout`.** v1's batch
  deadline **is** the first invocation's `timeout`; handoffs do not
  extend wall-clock. A separate `batch-timeout` knob is deferred.
- **Auto-re-dispatch on cap.** When the caps fire, the batch re-queues
  for the *next* producer trigger or a manual re-trigger; v1 does not
  invent a scheduler.
- **Tier 1 on `pi-stdio`.** No `usage` in the plain-text stream, no
  live steer — nothing to observe at the water-mark. A future
  transcript-length estimator is out of scope; `pi-stdio` remains the
  Tier-2 (manual-recovery) adapter.
- **A task-count gate (`tasks-per-session`).** Dropped (see
  **Configuration**): a count heuristic, superseded by the water-mark
  (now on `pi-rpc` and `pi-json`) and by overflow recovery elsewhere.
  Revisit only if an all-adapter pre-overflow handoff is needed.
- **A per-knot chain-cap knob (`max-continuations`).** v1 uses the
  global `MAX_CONTINUATIONS = 10` constant (mirrors `MAX_RETRIES`).
  Revisit only if a rig needs materially different chain lengths per
  knot.
- **A Knot-owned task/checklist format.** The checklist is rig-owned
  (format, location, and lifecycle defined by the rig design and the
  knot's prompt). Knot sees only the `TasksIncomplete` declaration — no
  `task-list.yml`, no `task_list.rs`, no Knot-side parsing of checklist
  content.
- **A declared self-subscription (`strand_source:
  event:<self|loom>:TasksIncomplete`).** One `strand_source` per knot
  (a declared self-subscription crowds out the work input); loom-level
  matching cross-fires between knots in the same loom. The implicit,
  knot-scoped, **universal** self-continuation (every knot, `occurred:
  true` only) replaces it.
- **Event-work-input task knots.** v1 task-bearing knots are
  filesystem-strand-triggered (the self-continuation lands in their
  strand dir). A task-bearing knot whose work trigger is an upstream
  event is a later extension.
- **Removing `pi-json`** — unchanged from 084 (side-by-side).
- **Parallel continuation hops.** Continuations run serially (the shared
  checklist + git index make parallel hops racy); the fan-out shape
  (forking) is the place for parallelism, deferred.
- **`knot-update` migration entry** — there is **no config key to
  add**: the behaviour change is scoped to (a) water-marked aliases
  (the water-mark payload becomes `HANDOFF_NOTE`, replacing 084's
  terminal `WRAP_UP_STEER` wind-down; the `# Subscriber Events` block
  gains the self `TasksIncomplete` entry) and (b) water-marked
  sessions (the tie-off must declare `TasksIncomplete` `true|false`).
  Non-water-marked aliases and sessions are unchanged. No migration
  required.

## Phases

**Status: ⬜ Planned.**

### Phase 0: Failing tests

1. `src/domain/events.rs` — the `# Subscriber Events` builder:
   - `subscriber_block_self_entry_gated_on_watermark` — a knot whose
     profile alias carries `ctx-wrap-up-limit` has the self
     `TasksIncomplete` entry (standard description = the handoff
     contract) in the assembled block; an alias without it has no
     entry and a block byte-identical to today's. Independent of any
     front-matter.
   - `context_wrap_up_steered_mechanism_round_trip` — the additive
     `mechanism` field on `ContextWrapUpSteered` serde-round-trips with
     default `steer` (084 records unchanged).
2. `src/application/usecases/process_strand.rs`:
   - `no_watermark_session_unchanged` — a session on an alias with
     **no** water-mark (or one that never crossed it) whose tie-off
     carries **no** `TasksIncomplete` block → behaves exactly as
     today: normal `KnotCompleted`, no continuation, no enforcement,
     no new loom event (regression guard). And a **water-marked**
     session that declares `occurred: false` → normal `KnotCompleted`,
     no continuation, no `TasksIncomplete` loom event (the explicit
     completion is the tie-off itself).
   - `tasks_incomplete_true_self_dispatch` — a response declaring
     `TasksIncomplete` `occurred: true` → the tie-off is `Produced`,
     the **knot-scoped implicit match** dispatches to the same knot
     (event file created in the knot's own strand dir), the stamps
     (deadline, `continuations: 1`, empty accumulated background) are
     on the dispatched file's front-matter, `LoomEvent::
     TasksIncomplete` is recorded alongside `KnotCompleted`, and the
     head event is late-removed exactly once.
   - `tasks_incomplete_false_no_dispatch` — a response declaring
     `occurred: false` → normal `KnotCompleted`, no continuation, no
     `TasksIncomplete` loom event (the explicit completion is the
     tie-off itself).
   - `tasks_incomplete_missing_enforced` — a **water-marked**
     session's tie-off missing the `TasksIncomplete` block →
     `KnotEventsMissing` (expected events include the self
     `TasksIncomplete`) + `[task-loop] handoff-missed`; a
     **non-water-marked** session missing the block → no enforcement
     (normal completion).
   - `self_dispatch_no_cross_fire` — two task-bearing knots in one
     loom: A's `TasksIncomplete` dispatches to A only, never B.
   - `duplicate_tasks_incomplete_one_continuation` — a run declaring
     two `occurred: true` `TasksIncomplete` events → exactly one
     continuation dispatch; the second is logged and dropped
     (serial-hops guarantee).
   - `continuation_timeout_from_deadline` — processing a
     `TasksIncomplete` event whose stamped deadline leaves 30s →
     `ExecutionContext.timeout == Some(30s)` (NOT the profile's
     `session_timeout()`); assert it never *exceeds* remaining.
   - `continuation_inherits_stamp_verbatim` — a hop whose incoming
     event carries deadline D and `continuations: 2`, declaring
     another `occurred: true` → the outgoing event carries D
     **byte-identical** and `continuations: 3` (never recomputed from
     the current invocation).
   - `background_accumulates_prompt_order` — hop 2's prompt =
     `listener+base` + `[B1]` + handoff₂ + ntc₂; hop 3's prompt =
     `listener+base` + `[B1, B2]` + handoff₃ + ntc₃, where `[B1, B2]`
     has `[B1]` as a byte-identical **prefix** and the per-hop
     sections always trail the accumulated block.
   - `continuation_deadline_exhausted_degenerate_tieoff` — deadline
     already passed (or `< MIN_REMAINING_SECS`) → **no agent call**; a
     Knot-authored degenerate terminal tie-off is written;
     `BatchIncomplete` (reason `deadline`) is logged; the event is
     **late-removed** (not re-queued); a later fresh dispatch gets a
     fresh budget.
   - `max_continuations_stops_chain` — `continuations ==
     MAX_CONTINUATIONS` (the global constant) and an `occurred: true`
     declaration → **no further dispatch**, `BatchIncomplete` (reason
     `caps`) logged, the checklist left for a later dispatch.
   - `chain_lifecycle_observable` — a water-marked run declaring
     `occurred: true` → the injection moment is `ContextWrapUpSteered`
     (mechanism `steer`), the response-return path records
     `KnotCompleted` + `LoomEvent::TasksIncomplete` + the
     `[task-loop] handoff` line; processing the stamped continuation
     event records `KnotProcessing` + the `[task-loop] continuation`
     line (hop, deadline, remaining); a water-marked run whose tie-off
     **misses** the block records `KnotEventsMissing` +
     `[task-loop] handoff-missed` (injection + response +
     continuation each observable).
   - `overflow_terminates_and_reenters` — a task-bearing knot whose
     run ends in terminal `ContextLimitReached` (Tier 2) → the strand
     fails/re-queues exactly as today, and a later dispatch of the
     same strand re-enters the knot with a fresh budget and the
     *ordinary* prompt (no Knot-side checklist parsing) — regression
     guard that Tier 2 needs no new machinery.
3. `src/adapters/pi_rpc.rs` (084 harness): with `ctx_wrap_up_limit`
   crossed → the `steer` message on stdin contains `HANDOFF_NOTE` text
   (wrap-up + the `TasksIncomplete` acknowledgement per its
   description), **not** the plain `WRAP_UP_STEER` — for **any** knot
   (no per-knot flag); the run records `LoomEvent::
   ContextWrapUpSteered` (mechanism default `steer`) once, with the
   084 fields unchanged.
4. `src/adapters/pi_json.rs` (new water-mark stop-resume harness):
   stream `usage` crosses `ctx-wrap-up-limit` → the process receives
   SIGINT (teardown grace), the invocation ends as a retryable
   **water-mark stop** carrying the session id; the retry invocation's
   args include `--session-id <id>` and the injected prompt contains
   `HANDOFF_NOTE` — for **any** knot; fire-once (a second crossing
   does not re-fire); no `ctx-wrap-up-limit` → no stop; the injection
   records `LoomEvent::ContextWrapUpSteered` (mechanism `stop-resume`).
5. `src/application/session_resume.rs` — **no metadata flag**:
   `LoomEvent::TasksIncomplete` is recorded from the parsed
   `occurred: true` tie-off event block in the response path (logged
   next to `log_wrap_up`), once, with the right fields; the
   `ContextWrapUpSteered` record (incl. the additive `mechanism`)
   round-trips alongside it.
6. `tests/` integration — a two-hop batch: setup run's tie-off
   declares `TasksIncomplete` `occurred: true` (with both context
   fields); the mock's second invocation sees the handoff block +
   accumulated background + `next-task-context` in the prompt, a
   `profile_timeout` strictly less than the profile's, and its tie-off
   carries no `TasksIncomplete` block (done — no water-mark crossed
   on the final hop). Assert two tie-offs, two commits, and
   `TasksIncomplete → KnotCompleted`.

### Phase 1: Domain — events + constant (no new front-matter)

**No `task-loop` front-matter, no `TaskLoop` type, no
`KnotDefinition`/`AgentConfig` change.** `src/domain/events.rs`:
`LoomEvent::TasksIncomplete` (reason `water-mark|voluntary`; optional
`tasks_done`/`tasks_remaining`) + `BatchIncomplete` (reason
`deadline|caps`) + the additive `mechanism` field on
`ContextWrapUpSteered` (serde default `steer`) (+ serde round-trips).
New global constant `MAX_CONTINUATIONS = 10` (mirrors `MAX_RETRIES`).
**No `task_list.rs`** — Knot never parses the checklist.

### Phase 2: The handoff contract in the prompt + consts + assembly

`src/domain/value_objects.rs` / `process_strand_helpers`:

- **The `# Subscriber Events` builder** adds the implicit self
  `TasksIncomplete` entry to every knot **whose profile alias carries
  `ctx-wrap-up-limit`** (gated: without the water-mark the note can
  never fire, the entry is absent, and the block is byte-identical to
  today's), with its description set to the new greppable const
  `TASKS_INCOMPLETE_DESCRIPTION` — the handoff contract (a wrap-up
  note asks you to stop ⇒ update the durable checklist/state + commit
  ⇒ `occurred: true` with pointer body + `next-task-context` +
  `background-additional` (thin; into the checklist if it must
  outlive the batch) if work remains, or `occurred: false` if nothing
  remains; a voluntary stop with work remaining ⇒ `occurred: true`
  the same way; a normal completion without a note ⇒ no block; emit
  `occurred: true` only when work genuinely remains **and** durable
  state points at it; optional `tasks-done`/`tasks-remaining`).
- `HANDOFF_NOTE` (078/084 greppable-const convention) — the water-mark
  payload: wrap up now, commit in-flight work, update your durable
  state, and acknowledge `TasksIncomplete` in your tie-off per its
  description (`occurred: true` + handoff fields if work remains;
  `occurred: false` if nothing remains). It is the steer payload on
  `pi-rpc` and the injected prompt on `pi-json` stop-resume; generic,
  never names the checklist.
- **Continuation prompt assembly** in `resolve_config_and_build`: the
  fixed order (static → accumulated background → handoff body →
  `next-task-context`) with the append-only, hop-labelled background
  stamp format (cacheable-prefix property).

No `AgentConfig` change — the drivers pick the water-mark payload by
the **alias's** `ctx-wrap-up-limit` (existing 084 plumbing), not by
any knot flag.

### Phase 3: Self-continuation dispatch + stamps

`dispatch_events_to_consumers`: the implicit **knot-scoped self-match**
(producer == consumer id ∧ event `TasksIncomplete` ∧
`occurred: true`) for every knot + delivery of the stamped event file
into the knot's own strand dir (v1: filesystem-strand-triggered task
knots). Stamping in the dispatch path: `batch-deadline-epoch`
(**inherited verbatim** from the incoming event, or
`now + profile_timeout` for the chain's first event), `continuations`
(incoming + 1, or 1 for the first), and the **accumulated
`background-additional`** (incoming accumulation + this event's
contribution, append-only). The **dedup guard**: at most one
continuation dispatch per invocation (first wins, rest logged +
dropped). `process_strand_helpers::handle_success`: a `Produced`
tie-off with a `TasksIncomplete` block → `occurred: true` takes the
self-continuation path (late removal + git commit unchanged — the
tie-off flows through the normal pipeline); `occurred: false` →
normal completion (no loom event, no dispatch); missing block →
normal on a non-water-marked completion, but on a **water-marked**
session `KnotEventsMissing` fires (the expected-events set includes
the self entry only then) + the `[task-loop] handoff-missed` line.
The response-return moment is observable (see
**Observability**): `KnotCompleted` + the `TasksIncomplete` loom event
+ the `[task-loop] handoff` line.

### Phase 4: The global timer + the degenerate tie-off

`resolve_config_and_build`: when the incoming event carries
`batch-deadline-epoch`, compute
`profile_timeout = batch_deadline_epoch.saturating_sub(now)` (remaining
global budget) and pass that to `execute_with_resume` instead of
`profile.session_timeout()`. The continuation session's start is then
logged — the existing `KnotProcessing` event + the `[task-loop]
continuation` service line (hop, deadline, remaining) — so the fresh
session is correlated to the batch it continues (see
**Observability**). If `< MIN_REMAINING_SECS`, do **not** call
the agent — write the **Knot-authored degenerate terminal tie-off**
(deferral; `TieOffStatus::Failed` with the `BatchIncomplete` note — no
new status in v1, the observability distinction lives in the loom
event), record `BatchIncomplete` (reason `deadline`), and let the
normal late-removal consume the event. This is the single change that
makes Tiers 1/2 obey the original clock.

### Phase 5: Max-continuations cap

In the continuation dispatch path: read `continuations` from the
incoming event; if `continuations >= MAX_CONTINUATIONS` (global
constant) and the agent declared `occurred: true`, suppress the
dispatch, log `BatchIncomplete` (reason `caps`), and leave the
checklist for a later dispatch.

### Phase 6: Tier-1 water-mark — `pi-rpc` steer

`src/adapters/pi_rpc.rs`: the 084 monitor already fires at
`ctx-wrap-up-limit`. It now sends `HANDOFF_NOTE` (wrap up, commit,
update the durable state, acknowledge `TasksIncomplete` `occurred:
true` with the two context fields if work remains, `occurred: false`
if nothing remains) **instead of `WRAP_UP_STEER` — for every knot**;
the terminal wind-down is subsumed (a water-marked session with no
work left simply declares `occurred: false` and stops). Reuse the
fire-once, `get_session_stats`, null-skip, teardown rules unchanged;
record `ContextWrapUpSteered` (mechanism `steer` by default) with the
084 fields.

### Phase 7: Tier-1 water-mark — `pi-json` stop-resume

`src/adapters/pi_json.rs`: a monitor on stream `usage` (per-message
input/total tokens) with the 084 fire-once / `ctx-wrap-up-limit` /
null-skip discipline. On crossing: SIGINT + bounded teardown grace
(force-kill), the invocation ends as a **retryable water-mark stop**
(session id already captured). The session-resume retry re-invokes
with `--session-id` + the injected `HANDOFF_NOTE` — for **every**
knot. The injection records `LoomEvent::ContextWrapUpSteered`
(mechanism `stop-resume`) — the injection moment is a loom event on
this adapter too, not only a service-log line. `pi-stdio` unchanged
(no `usage` → no monitor → Tier 2 only; the alias-level fact "no
water-mark ⇒ no graceful handoff" is documented, not warned per knot).

### Phase 8: Tier-2 recovery — no Knot changes (verify + document)

The overflow/failure path (080 `ContextLimitReached`, 081 timeout)
already writes a terminal tie-off and consumes the event; the knot is
idempotent, so the re-entered agent resumes from the checklist — the
same resume the Tier-1 chain does, except the "handoff" is the
checklist itself rather than an event body (the agent, not Knot, is the
handoff author). Verify with the Phase-0 `overflow_terminates_and_
reenters` test; document the re-dispatch path and the Tier-2
chain-bounding story (fresh dispatch per hop; `pi-stdio` = manual-
recovery adapter). No new prompt construction, no checklist parsing.

### Phase 9: Observability

Wire the three lifecycle moments (see **Observability**):
`ContextWrapUpSteered` (additive `mechanism`) from the `pi-rpc` steer
and the `pi-json` water-mark stop-resume + `LoomEvent::
TasksIncomplete` (reason `water-mark|voluntary`; optional visibility
fields) from the `occurred: true` response path + `BatchIncomplete`
(reason `deadline|caps`) from the dispatch/cap/degenerate paths + the
existing `KnotEventsMissing` (expected events include the self
`TasksIncomplete` for water-marked sessions) + the 082 system events
(all best-effort, never fail the strand); service-log lines (`handoff`,
`handoff-missed`, `continuation`, `batch-incomplete`); `session_resume` logging next to
`log_wrap_up`.

### Phase 10: Wiring + docs + version

- `cargo test` full suite; `cargo clippy --all-targets` clean (new code).
- **Scratch probes** (temp dir, per AGENTS.md — no rig run here):
  confirm a self-dispatched `TasksIncomplete` re-enters the same knot;
  confirm a continuation's `--mode rpc` run inherits the shrunken
  `profile_timeout`; confirm the hop-2 prompt's background block is a
  byte-identical prefix of hop-3's.
- `docs/concepts.md` — the graceful-handoff paragraph (overflow is the
  denominator; handoff supersedes compaction for task work; the two
  tiers — steer *and* stop-resume; the rig-owned checklist as
  keystone; the default-on contract: `TasksIncomplete` is a declared
  self-event for every knot, listed on water-marked aliases, and a
  water-marked session's tie-off declares it `occurred: true|false`
  explicitly (the water-mark note is the decision moment; a completion
  without the note carries no block); the two carried context fields + the
  cacheable-prefix prompt order; Knot stays checklist-blind; "fresh
  context, never a fresh budget").
- `docs/configuration/knot-structure.md` — **no new front-matter**;
  the `TasksIncomplete` self entry in the `# Subscriber Events` block
  (listed on water-marked aliases; the contract description;
  water-marked sessions must declare `occurred: true|false` — a note-
  driven stop with no work left declares `false`; a completion without
  the note carries no block); reuse of `ctx-wrap-up-limit` as the
  water-mark for **both** `pi-rpc` and `pi-json` (payload
  `HANDOFF_NOTE`, replacing `WRAP_UP_STEER`); the implicit self-
  continuation (no declared subscription); the two context fields;
  the global `MAX_CONTINUATIONS`. No `tasks-per-session`; no Knot-
  owned checklist format.
- `docs/troubleshooting.md` — "task knot keeps re-dispatching /
  `BatchIncomplete`" → the batch outgrew one budget; it resumes from
  the checklist on the next dispatch; tune `timeout` (the cap is the
  global constant). "Batch died after one hop" → setup consumed the
  batch budget; raise `timeout`. "Water-mark fired but the batch
  stalled" (`ContextWrapUpSteered` + `KnotEventsMissing` missing
  `TasksIncomplete` — the `[task-loop] handoff-missed` line) → missed
  declaration; check the checklist, re-trigger. "Knot declares
  `occurred: true` but the checklist never moves" → rig quality: the
  checklist/commit discipline (or the `occurred: true` discipline —
  work genuinely remaining *and* durably pointed at) is the knot
  prompt's job; each spurious `true` costs one hop. "`pi-stdio` task
  knot" → manual re-trigger per hop; use `pi-rpc`/`pi-json` with a
  water-mark for automatic chains.
- `docs/release-notes.md` — v0.43.0: Graceful Task Handoff (default-on
  handoff contract, checkpointed continuation chains, global-timer
  discipline, `pi-json` water-mark stop-resume, explicit
  `TasksIncomplete` tie-off declaration).
- Skills: `knot-create` (no new front-matter; the subscriber-block
  self entry; the implicit self-continuation; the water-mark fact —
  aliases without `ctx-wrap-up-limit` have no graceful handoff; no
  `tasks-per-session`), `knot-design` (the checklist conventions:
  scope-once, work-the-checklist, split-if-too-large, the
  planner→phase-runner example, the durable-vs-carried layering rule —
  the rig-side termination guarantee — and the `occurred: true`
  discipline: declare work remaining only when it genuinely remains
  and durable state points at it), `knot-inspect` (reading the
  `TasksIncomplete` declaration in a tie-off — `true` (with the
  carried fields in the dispatched event's front-matter) vs
  `false` (explicit batch complete after a note-driven stop) vs
  absent (a completion without the note); reading a checklist —
  whatever file the knot's prompt names; Knot stores no checklist of
  its own),
  `knot-update` (no key to add — the behaviour change: water-marked
  aliases get the handoff note instead of the terminal wind-down and
  the self entry in the subscriber block, and water-marked sessions'
  tie-offs must declare `TasksIncomplete` `true|false`; a session that
  never crosses the water-mark is unchanged, and a non-water-marked
  alias's prompt is byte-identical to today's). Deploy touched sub-skills
  to `~/.agents/skills-library/` with diff verification per AGENTS.md.
- Bump `Cargo.toml` `0.42.0 → 0.43.0`; `cargo install --path .`.
- `project/plans/master-plan.md` — row 86.

## Notes — design rationale

- **Why default-on, with no flag?** The contract rides on two existing
  delivery points — the `# Subscriber Events` entry (water-marked
  aliases, like any declared event) and the water-mark steer (the
  decision moment) — so a static opt-in flag would gate nothing the
  agent's runtime declaration doesn't already decide: a session asked
  to wrap up declares `occurred: true` or `occurred: false`, and only
  `true` dispatches. The costs are the subscriber-block entry on
  water-marked aliases and the declaration in a water-marked
  session's tie-off — a session that never crosses the water-mark is
  untouched (no entry on non-water-marked aliases, no block on a plain
  completion). The water-mark payload change subsumes the terminal
  wind-down: a water-marked session with no work left declares
  `occurred: false` and stops, which is the wind-down.
- **Why the subscriber block (the existing tie-off event rule) rather
  than a separate duty block?** The rule already defines the format
  (one explicit `occurred: true|false` block per event, required
  fields, `false` = acknowledged-but-not-dispatched) and already has
  the enforcement (`KnotEventsMissing` when a declared event is
  missing from a tie-off). Listing `TasksIncomplete` as a (self-)
  subscriber entry buys that format and enforcement for free and adds
  no second prompt mechanism. The *obligation* is scoped to the
  water-mark, not the prompt: the entry is listed only on water-marked
  aliases (the format must be known before the note can fire), but the
  declaration is **expected only when the note fired** — the response
  path knows (the wrap-up record is right there), so
  `KnotEventsMissing` fires precisely on a water-marked session that
  never declared, and a session that finished without the note owes
  nothing — plain completion, no block. That keeps "batch complete" a
  positive declaration exactly where it matters (the session that was
  asked to wrap up) without putting an acknowledgement in every
  tie-off.
- **Why handoff supersedes compaction for task work?** Compaction
  summarizes the *context* — lossy, and the agent continues on
  degraded memory. Handoff starts a *fresh session* whose context is
  `instructions + accumulated background + handoff + brief`; the
  handoff **points at durable artifacts** (checklist, committed
  files) rather than summarizing the transcript. It is lossless-ish
  (the agent decides what matters and points at committed truth),
  bounded (fresh session), and progress-carrying (the checklist
  advances each hop).
- **Why drop the task-count gate?** It bounded *count*, not *size* — a
  session that did K small tasks had plenty of room, and one that did
  two large tasks overflowed anyway; it was always the water-mark's
  backstop, never its replacement. It also forced every task knot to
  maintain and gate on a counter the agent had to reason about, for a
  coarse bound the context water-mark renders redundant where context
  is observable. Where context is *not* observable (`pi-stdio`), the
  honest options were overflow-recovery (which the idempotent knot +
  durable checklist already gives) or a heuristic that pretends K tasks
  fit. This plan takes the honest option and documents `pi-stdio` as
  the manual-recovery adapter.
- **Why is the checklist rig-owned, not Knot-owned?** Knot stays
  checklist-blind: it never reads the checklist, and it needs no
  checklist content — only the explicit declaration in the tie-off
  (the seam), the stamps, and the carried context fields. The
  checklist's shape is domain-specific (phase checklists, validation
  combos, review items), and the rig designer — often via a *different*
  knot (planner → phase runner) — is best placed to design it. A
  Knot-owned format (`086`'s earlier `task-list.yml` draft) would be a
  migration surface (the 082/085 lesson) for zero functional gain, and
  would force every rig into one checklist shape. Idempotent knots +
  durable files are already the rig's recovery story (080/081); this
  plan makes the *graceful* path (the water-mark + `TasksIncomplete`
  chain) as cheap for Knot as the recovery path.
- **Why an implicit self-continuation instead of a declared
  subscription?** A knot has exactly one `strand_source` — its only
  declared input. A declared self-subscription (knot-level
  `event:<self>:TasksIncomplete` or loom-level
  `event:<own-loom>:TasksIncomplete`) would *replace* the work input
  (producer strands, manual re-triggers) that starts batches — a
  task-bearing knot needs both inputs. Loom-level matching
  additionally cross-fires: every knot in the loom subscribed to
  `TasksIncomplete` receives *every* knot's handoff. The implicit
  knot-scoped match (producer == self, universal, `occurred: true`
  only, delivered into the knot's existing input) keeps the declared
  input intact, needs no new front-matter, and is knot-scoped by
  construction. (The *prompt* and *enforcement* sides of the
  self-consumption ride on the standard `# Subscriber Events`
  machinery — one entry in the list, one expected acknowledgement —
  not on a separate mechanism.)
- **Why `pi-json` stop-resume instead of running to overflow?** The
  pieces already exist — per-message `usage` + `session_id` in the
  stream, the `--session-id` resume-retry path — so Tier 1 costs a
  monitor + a SIGINT + one injected prompt, and stays *within one
  invocation* (same `start`), leaving the global-budget arithmetic
  untouched. The water-mark leaves `contextWindow − reserve − limit`
  of headroom, so the resumed wrap-up normally fits; the pathological
  case (wrap-up overflows) degrades to terminal Tier 2. `pi-stdio`
  cannot get this (no `usage` in a plain-text stream) — hence the
  adapter ladder, not a uniform mechanism.
- **Why the prompt order — accumulated background before the per-hop
  sections?** Providers cache the prompt *prefix*. The accumulated
  background is the only monotonically growing section; placing it
  before the per-hop variable sections (handoff body,
  `next-task-context`) and keeping it append-only/deterministic makes
  each hop's background block a byte-identical prefix of the next —
  so hop N+1 re-uses the cached prefix through all previous hops'
  contributions and pays for only its own tail. A variable section
  ahead of the background would bust the cache every hop and
  reprocess the whole accumulated block cold. This is also why the
  stamp format is append-only (hop-labelled, emission order, never
  rewritten): reordering or reformatting would silently destroy the
  prefix.
- **Why is `background-additional` chain-scoped while the checklist is
  the durable layer?** The accumulation rides the event chain — it is
  cheap prompt-injected context, not a file. A batch that spans
  dispatches starts fresh (no carried state), so anything that must
  outlive the batch has to be *durable*: in the git-tracked checklist.
  The split — durable state → checklist; must-know facts → carried
  (and into the checklist if they must outlive the batch) — keeps the
  cacheable prefix cheap and the truth durable, without Knot ever
  reading either.
- **Why a pointer handoff, never a re-statement?** A re-statement is
  as big as the context (defeats boundedness) and as lossy as
  compaction (if summarized). A pointer survives a thin handoff
  because it resolves to the checklist + committed files — the rig's
  source of truth (the same reasoning as 084's "reconstruct from the
  filesystem").
- **Why is `profile_timeout` the single lever for the global timer?**
  `execute_with_resume` already treats `profile_timeout` as the global
  budget within one invocation (`start.elapsed()` vs it, per-attempt =
  `profile_timeout - elapsed`). So *deriving it from the batch
  deadline* at one point (`resolve_config_and_build`) makes the whole
  retry chain obey the original clock with no other change. Stamps
  must be **inherited verbatim** (never recomputed from the current
  invocation): recomputation re-extends the deadline by each hop's
  startup/queue-wait drift — a silent per-hop budget reset. This is
  the concrete embodiment of "the timer is not reset for
  `TasksIncomplete` / overflow."
- **Why is `MAX_CONTINUATIONS` a separate cap from the deadline?**
  They protect different resources: the deadline bounds **wall-clock**;
  the count bounds **spawns/commits/spend**. A fast chain making tiny
  progress can hit a huge session count with time left; a slow chain
  can exhaust the clock in two hops. Both caps are needed, both fire a
  `BatchIncomplete` and defer to a later dispatch. Both bound the
  *automatic* chain only; Tier-2 chains span fresh dispatches and are
  bounded by the split rule + the operator instead. The global
  constant (vs a per-knot knob) is v1 scope: one chain length for the
  fleet, mirroring `MAX_RETRIES`.
- **Why is `TasksIncomplete` an explicit tie-off declaration (and a
  dispatched event) rather than a new `TieOffOutcome` or a metadata
  flag?** The session genuinely *succeeded* (full tie-off,
  `Produced`); reusing `dispatch_agent_events` keeps tie-off writing,
  git versioning, late removal, and event enforcement untouched, and
  makes the continuation a first-class event in the queue
  (observability, idempotency, and the at-least-once guarantee come
  for free). Declaring it through the existing tie-off event rule
  (explicit `occurred: true|false` in a water-marked session's
  tie-off) makes the batch outcome a positive statement exactly where
  the wrap-up was asked — `false` is the "batch complete" declaration,
  `true` the continuation trigger, and a missing block on a
  water-marked session is an enforcement violation
  (`KnotEventsMissing`), not a silent inference; a non-water-marked
  completion carries no block by design.
- **Why is the split rule rig-design, not Knot-injected?** Knot cannot
  verify checklist content (it never reads the checklist), so it cannot
  enforce decomposition — the contract description and the hard caps
  bound the worst case, and `knot-design` documents split-if-too-large
  as a rig-design obligation for task-bearing knots. That keeps Knot's
  task surface at exactly one event.
- **Why is the batch deadline the first `timeout`, not a new larger
  budget?** "New sessions get a fresh timer" is the bug; "the original
  timer is obeyed" is the fix. Making the batch's wall-clock equal a
  single session's `timeout` means a handoff buys a fresh *context*
  but never a fresh *budget*. Batches that legitimately need more
  **span dispatches** (re-queued, resumed from the checklist), which
  is the correct place to grant more time — not an in-chain reset.
  Note the batch deadline *includes* the setup session's wall-clock:
  scope-once work counts against the batch budget (intentional — the
  batch is one session's time; rigs that need more raise `timeout`).
- **Why serial hops, not parallel?** The checklist and git index are
  shared; parallel hops would race them. The dedup guard (one
  continuation per invocation) enforces seriality at the source. The
  fork shape (deferred) is where parallelism belongs (independent
  session files, disjoint commits).

## Implementation Status: ⬜ Planned
