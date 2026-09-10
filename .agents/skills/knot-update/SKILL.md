---
name: knot-update
description: "Record format changes between Knot binary versions. When a project updates its Knot binary, this skill tells the agent what changed in project documents (profiles, knots, looms) and how to migrate them. Contains a versioned changelog with migration instructions for each breaking change. USE FOR: update knot, knot version change, knot migration, knot changelog, migrate knot documents, knot format change, knot upgrade, knot breaking change, profile format change, knot file migration, loom migration. DO NOT USE FOR: creating looms (use knot-create), modifying looms (use knot-create), initialising a rig (use knot-init), inspecting state (use knot-inspect), fixing bugs (use project-bugfix)."
license: MIT
metadata:
  author: Knot Team
  version: "1.21.0"
  compatibility: "Knot 0.41.0+"
---

# Knot Update Skill

Record and communicate format changes between Knot binary versions. When a
project updates its Knot installation, this skill provides the agent with a
versioned changelog of document format changes and step-by-step migration
instructions.

This is a **reference skill** — it does not define a workflow to execute.
Instead, it is read when a project updates Knot so the agent knows what
document changes are required.

---

## Core Philosophy

### Format Changes Are Breaking by Default

Knot reads `.md` files with YAML frontmatter. Any change to the
frontmatter schema or body semantics is a breaking change for existing
project documents. Projects that have been running with Knot will have
documents in the old format that must be migrated.

This skill ensures:

- **Every version is documented** — even small format tweaks
- **Migration is mechanical** — search patterns and replacements are
  explicit, not described in prose alone
- **Projects can self-serve** — the agent reads this skill and applies
  migrations without external guidance

### How This Skill Is Used

1. A project updates its Knot binary (e.g. `cargo install --path .` or
   downloads a new release).
2. The agent reads this skill file to see what changed since the
   project's last Knot version.
3. For each changelog entry newer than the project's current version,
   the agent applies the migration instructions to the project's
   `rig/profiles/*.md` and `rig/*-loom/*.md` files.
4. The agent verifies the migrated files by checking `rig/state.json`
   (Knot must be running).

---

## Changelog

Entries are listed newest first. Each entry specifies the Knot version,
date, and migration instructions for affected document types.

---

### Event-Source Self-Continuation + Queue-Wait-Exempt Budget (Knot 0.44.0, 2026-09-10)

**What changed:** two changes to the `TasksIncomplete` self-continuation
chain (plan 087, building on 086).

1. **Event-source knots now self-continue.** v1 (0.43.0) left event-source
   knots (`strand-dir: event:<producer>:<EventId>`) out of the
   self-continuation: the dispatcher had no target for them, so a batch
   paused with work remaining. The stamped continuation is now delivered
   into the knot's **existing event dispatch dir**
   (`tie-offs/<rig>/<loom>/<event-id>/`) — the dir the knot's existing
   watcher already watches (bound to `(loom, knot)`) — so the watcher fires
   `StrandEvent::Created` for that knot and the batch re-enters through the
   normal dispatch pipeline. No new subscription, queue, or `strand-source`.
   Filesystem-strand knots are unchanged (the continuation still lands in the
   knot's own strand dir).
2. **Queue-wait-exempt batch execution budget.** 0.43.0 stamped an absolute
   `batch-deadline-epoch` and, at dequeue, subtracted elapsed wall-clock
   (`remaining = deadline − now`) — so a lengthy event queued *between* the
   handoff and the continuation's dequeue eroded the budget. The continuation
   front-matter now carries the **remaining execution budget in seconds**
   (`budget-secs`) plus the batch's absolute origin (`batch-start-epoch`).
   Only execution decrements the budget (`budget-secs = incoming −
   execution_secs`); queue wait never does. The whole batch (all hops,
   including the first) shares one total execution budget of
   `profile_timeout`, never reset at a handoff.

**Continuation front-matter (a Knot-owned runtime artifact, never authored by
the agent):**

| Field | 0.43.0 (086) | 0.44.0+ |
|---|---|---|
| `continuations` | present | present (unchanged) |
| `batch-deadline-epoch` | absolute deadline (eroded by queue wait) | **removed** |
| `budget-secs` | — | remaining execution budget in seconds (only execution decrements) |
| `batch-start-epoch` | — | absolute origin (first handoff; observability only) |

**Affected documents:** none — the continuation file is a **Knot-owned
runtime artifact** (written into the event dispatch dir / strand dir, never
authored by the agent or a rig document). No profile, knot, loom, or
`models.yml` format changes.

**Migration: none required.**

- Existing rigs are unchanged; no file edits.
- A continuation file written by a 0.43.0 build (carrying only
  `batch-deadline-epoch`) is read by 0.44.0 with a one-time **read-only
  shim**: the budget is derived from the deadline at read time
  (`budget_secs = deadline − now`). New continuations never write
  `batch-deadline-epoch`, so the shim is a single-release bridge and is
  removed afterwards. Any in-flight 0.43.0 continuation simply re-runs with a
  budget derived from its deadline.
- The `[task-loop] handoff` service-log line now prints the stamped remaining
  budget (`remaining=<B>s`) and the delivery path (`source=filesystem|event`);
  the `TasksIncomplete` / `BatchIncomplete` loom events carry `budget-secs`
  + `batch-start-epoch`.

---

### `pi-rpc` Runner + Context Wrap-Up Steering (Knot 0.42.0, 2026-09-07)

**What changed:** a new optional agent runner, `agent-adapter: pi-rpc`,
speaks pi's `--mode rpc` JSONL protocol over stdin/stdout. It watches
the session's live context usage and, when it crosses a per-alias
`ctx-wrap-up-limit` (tokens), sends a one-shot `steer` (the wrap-up
prompt) at the next turn boundary so the agent commits, records progress,
and ties off before the context runs out. The `pi-json` runner is
unchanged and remains the default.

**New config keys / values (additive):**

- **`agent-adapter: pi-rpc`** — a new value for the **rig-level**
  `agent-adapter` key in `rig/.workspace-agent-config.yaml` (one adapter
  per rig, shared by all its knots). Selects the RPC runner. `pi-json`
  and `pi-stdio` are unchanged.
- **`ctx-wrap-up-limit: <tokens>`** — a new **per-model-alias** key in
  `rig/models.yml` (v1 is alias-level only; a direct-spec profile with no
  `model-ref` has no limit, feature off). When set (> 0) *and*
  `agent-adapter: pi-rpc`, the runner steers the session to wrap up once
  the sampled context tokens cross the limit. A value of `0` disables
  steering (filters to `None`). Steering is one steer per run and fires
  regardless of whether pi's own auto-compaction is enabled. Under a
  non-rpc adapter the limit is ignored (the runner logs a one-shot
  warning that the adapter cannot steer mid-run).
- **`ContextWrapUpSteered`** — a new one-shot loom event (loom-log) and
  new `wrap_up` field on `AgentInvocationMetadata` recording the steer
  (`context_tokens`, `limit`). A `[KNOT][EVENT]` line is rendered in the
  service log when it fires.

**Affected documents:** none require migration — the change is additive.

**Migration: none required.**

- Existing `rig/.workspace-agent-config.yaml` and `rig/models.yml`
  files are unchanged and continue to work. Both new keys are opt-in.
- To **opt in**, set `agent-adapter: pi-rpc` in
  `rig/.workspace-agent-config.yaml` (rig-level — it applies to every
  knot in the rig) **and** add `ctx-wrap-up-limit: <tokens>` to the
  model alias in `rig/models.yml` (below the model's effective
  compaction point, e.g. a few thousand tokens under the
  reserve-aware limit). Both are required for steering.
- To **subscribe** to the steer, set a knot's `strand-dir` to
  `event:<producer>:ContextWrapUpSteered` (system-event subscription,
  Knot 0.41.0+).
- **Recommended placement:** the adapter is a **per-rig** choice, so put
  `pi-rpc` on rigs whose work is long authoring / building (where context
  exhaustion is the real risk). The steer prompt is wasted overhead on a
  short consumer/review-only rig — keep those on `pi-json`.

**Deferred next change:** removal of the `pi-json` runner, once
`pi-rpc` has proven itself in rig service.

---

### Root-Anchored Rig `.gitignore` Entry (Knot 0.41.1, 2026-09-07)

**What changed:** the rig exclusion Knot appends to the parent repo's
`.gitignore` is now root-anchored — `/{basename}/` (e.g. `/rig/`) instead
of the unanchored `rig/`. Git treats a slash-less pattern as matching at
**any depth**, so the old `rig/` line also ignored nested directories such
as `tie-offs/rig/` — the project-versioned runtime data. The anchored
pattern matches only the top-level rig directory.

**Migration (automatic, no agent action):** on the first startup with
0.41.1+, `ensure_rig_repo()` rewrites any existing bare `{basename}/`
line to `/{basename}/` in place — the marker and every other line are
preserved. An already-anchored line is left untouched (idempotent).
Rig documents (looms, knots, profiles) are unaffected.

**Operational note:** the logged untrack command for pre-existing
tracked rigs now reads `git rm -r --cached /rig/` (root-anchored
pathspec); both the old and new forms work.

---

### System Event Subscriptions — Every Log Event Is Dispatchable (Knot 0.41.0, 2026-09-07)

**What changed:** every system event Knot writes to the loom-log /
rig-log is now also **dispatchable to subscriber knots** via the
existing `event:` `strand-dir` URI. Two new producer-token positions
join the existing knot- and loom-level positions:

- **Wildcard** `*` — `event:*:<EventId>` — match any knot in the rig.
- **Rig-level** `<rig-id>` — `event:<rig-id>:<EventId>` — a rig-scoped
  event (currently `QueueIdle`), using the rig's ID as the producer
  token.

The subscribable system events (by scope):

- **Run outcome — knot-scoped:** `KnotProcessing`, `KnotFailed`,
  `KnotCompleted`, `KnotEventsMissing`, `TimeoutExceeded` (knot-scoped
  despite living in the rig-log — it carries loom/knot/strand).
- **Run outcome — loom-scoped:** `StrandIgnored`, `StrandSkipped`,
  `StrandProcessed`.
- **Retry / session — knot-scoped, per attempt:** `SessionResumed`,
  `KnotEmptyResponse`, `AgentInactivity`, `ContextCompacted`.
- **Loom lifecycle — loom-scoped:** `LoomStarted`, `LoomStopped`,
  `KnotParseWarning`.
- **Knot lifecycle — knot-scoped:** `KnotRegistered`,
  `KnotDeregistered`, `DirectoryCreated`.
- **Rig lifecycle — rig-scoped:** `QueueIdle`.

Two behavioural rules:

- **Self-exclusion:** a system event is **never** dispatched back to
  the knot that produced it. A knot's own `KnotFailed` / `KnotCompleted`
  does not re-trigger it (the only sane "self" consumer of a terminal
  failure would be an infinite loop). Agent-emitted events are
  unaffected — a knot may still deliberately subscribe to its own agent
  events.
- **Per-attempt events fan out once per attempt, not per run.** A
  run that retries 3× fires a `SessionResumed` / `AgentInactivity` /
  `KnotEmptyResponse` / `ContextCompacted` subscriber 3×. Subscribers
  must be **idempotent** under re-delivery.

`EventsDispatched` is intentionally **not** dispatchable (dispatching
it would require recording its own dispatch).

**Affected documents:** none — no profile, knot, loom, or tie-off
format change. The change is additive; existing looms are unaffected.

**Migration: none required.**

- Existing `event:<knot>:<EventId>` and `event:<loom>:<EventId>`
  agent-event subscriptions are unchanged.
- To subscribe to a system event, set a knot's `strand-dir` to
  `event:<producer>:<SystemEventId>` where `<producer>` is a knot ID,
  a loom ID, `*`, or the rig ID (for `QueueIdle`). See the
  `knot-create` skill's **System Events** catalog for scopes and
  payload keys.
- Failure / retry subscribers must be idempotent (per-attempt
  re-delivery) and cross-knot failure→retry loops must be bounded
  (see the `knot-design` skill's **System-Event Subscribers: Loop
  Discipline**).

---

### Consolidated Service Log + Change-Driven `state.json` (Knot 0.41.0, 2026-09-07)

**What changed:** two observability changes — no project document
format changes.

1. **The per-run log files are gone.** The per-loom `.loom-log` and the
   rig-level `.rig-log` JSONL files (and the startup clear they
   carried since 0.34.0) are removed. Every domain event is emitted
   as a single-line `[KNOT][EVENT]` record on the service's stderr —
   e.g. `LoomStarted loom=X`, `KnotProcessing loom=X knot=Y
   strand=…`, `KnotCompleted … tie-off=…`, `TimeoutExceeded …`,
   `QueueIdle`. The `knot-start` skill appends that stderr to
   `tie-offs/<rig>/knot-service.log`, which is the durable operational
   record across restarts (filter per loom with
   `grep 'loom=<id>'`). Run activity is in-memory per process;
   **nothing is cleared at startup anymore**.
2. **`state.json` is written on change.** The state-writer task derives
   the state on every 5-second tick and diffs it against the last
   written state (ignoring `updated_at`, which is the only field that
   would differ); a no-op tick writes nothing, so `state.json`'s mtime
   is no longer a liveness signal in steady state. The baseline is
   written immediately at startup, so a fresh `updated_at` still proves
   a recent start; steady-state liveness is the service pidfile
   (`kill -0`). Each real write logs one or more `[KNOT][STATE]` lines
   (`initial snapshot` / `change knot loom/knot: status
   idle→completed` / `change queue+ …` / …).

**Affected documents:** none — no profile, knot, loom, or tie-off
format change. The `state.json` schema is unchanged; `updated_at`
now records the last *actual* change, not the last 5-second tick.

| Artifact | Before 0.41.0 | 0.41.0+ |
|---|---|---|
| `.loom-log` / `.rig-log` | per-run JSONL files, cleared at every startup | removed — one-line `[KNOT][EVENT]` records on stderr; the appended `knot-service.log` is the durable record |
| `state.json` writes | rewritten every 5-second tick regardless of change | written only when the state actually changes (baseline immediately at startup) |
| `state.json` mtime | fresh every 5 seconds while running | unchanged in steady state; steady-state liveness is the pidfile |
| Legacy `.loom-log` / `.rig-log` files (0.31.0 migration) | cleared at every startup | inert — not read, not cleared, not written |

**Migration: none required.**

- No document edits. Legacy `.loom-log`/`.rig-log` files left behind
  by earlier versions are inert; they may be deleted at leisure.
- Workflows that read the retired log files (activity tailing, event
  counting, `QueueIdle` monitoring) must now grep the service log:
  `grep '\[KNOT\]\[EVENT\]' tie-offs/<rig>/knot-service.log`.
- Workflows that used `state.json`'s mtime as a liveness probe must use
  the pidfile instead (`kill -0 $(cat .knot-service.pid)`); the state
  baseline at startup is still valid for verifying a fresh start.

---

### Tie-Off Sections Record the Pi Session ID (Knot 0.39.0, 2026-09-01)

**What changed:** Every tie-off section written in append mode may now
carry an optional `session:` metadata line, placed directly after
`Timestamp:` and before the event-trigger metadata (`event:`,
`source:`, `original_strand:`). The line records the pi session ID
that produced the section, so a reviewer can resume or inspect the
exact conversation with `pi --session-id <id>` (sessions live under
`~/.pi/agent/sessions/`). The line is present whenever a session ID
was captured — JSON-adapter runs on success and on failure with a
parsed session line — and is **omitted entirely** otherwise (stdio
adapter, unparseable output, pre-session errors), so unaffected
sections keep their exact previous shape.

**No migration required:** old tie-off files are untouched; the
section parser treats `session:` like the other body-metadata lines
(it lands in the parsed section body). New readers tolerate its
absence.

**Affected documents:** none — no project document (profile, knot,
loom) format change. Tie-off files are rig output, not rig input.

| Artifact | Before 0.39.0 | 0.39.0+ |
|---|---|---|
| Tie-off section header | `Timestamp:` followed by event metadata or `---` | Optional `session: <id>` line directly after `Timestamp:` (present when the pi session ID was captured) |

---

### Enforcement Follow-Up No Longer Dispatches Acknowledgements (Knot 0.38.2, 2026-09-01)

**What changed:** When a knot emits no event blocks but consumers
are listening, the enforcement follow-up re-enters the session and
dispatches any events found in the follow-up response. Until 0.38.1,
that follow-up path skipped the `occurred: true` filter the main
dispatch path applies: an `occurred: false` acknowledgement block
("nothing happened") in the follow-up response was dispatched to
consumers as a real event file, creating spurious queue work.
0.38.2 moves the filter into the shared dispatch choke point, so
acknowledgements are never dispatched from any path. Acknowledgements
still count as an enforcement response — a follow-up that only
acknowledges does not log a second `KnotEventsMissing`.

**Affected documents:** none — no project document (profile, knot,
loom, tie-off) format change.

| Artifact | Before 0.38.2 | 0.38.2+ |
|---|---|---|
| Enforcement follow-up emits `occurred: false` only | acknowledgement dispatched as a real event file to each matching consumer | no dispatch; acknowledgement counts for enforcement, no second `KnotEventsMissing` |
| Main-path `occurred: false` handling | unchanged (already filtered) | unchanged |
| Profile/knot/loom/tie-off formats, event queue schema | — | unchanged |

**Migration: none required.**

---

### Context Overflow — Compact and Continue, with Loom-Log Visibility (Knot 0.38.0, 2026-08-26)

**What changed:** pi's built-in compaction is now enabled for rig
sessions via a project-level `.pi/settings.json`
(`{"compaction": {"enabled": true}}`) — `knot-init` seeds the file at
rig initialisation (create-if-absent only; an existing settings file
is never overwritten). Knot records each successful compaction
observed in an invocation's JSON stream as a new `ContextCompacted`
loom-log entry (`reason` — `"overflow"` = the context limit was hit,
`"threshold"` = proactive; `tokens_before`; `session_id`; `attempt`),
and fails the strand immediately with `context limit reached: …` when
pi's own compact-and-retry could not recover a terminal overflow (new
`PortError::ContextLimitReached` — not resumable: no session-resume
retries, `Failed` tie-off, no rig-log timeout). No profile, knot,
loom, or tie-off format change.

**Why:** with compaction disabled, an over-full context surfaced as
the model's overflow error; pi exits 0 in JSON mode in that case, the
final-response filter drops the error message, and the session-resume
nudge loop re-entered the *same* over-full session until all 10
retries burned — none could succeed, and every retry clocked up
against the budget. Compaction-on lets pi recover in-process
(compact-and-continue); the loom entries make context pressure
visible so prompts and strand scope can be narrowed; the fail-fast
removes the pure clock-up when the kept context itself cannot fit.

**Affected documents:** none — no project document (profile, knot,
loom, tie-off) format changes.

| Artifact | Before 0.38.0 | 0.38.0+ |
|---|---|---|
| `.pi/settings.json` at the project root | not created by knot-init | seeded with `{"compaction": {"enabled": true}}` at init, only if absent |
| Context overflow in a rig session | nudge loop re-enters the same over-full session; all retries burn → `no final response: … (session resume exhausted)` | pi compacts and continues in-process; terminal overflow fails immediately with `context limit reached: …` |
| Loom-log compaction visibility | none | `ContextCompacted` entry per successful compaction (reason, tokens_before, session_id, attempt) |
| Rig-log on terminal overflow | only reachable via budget exhaustion (`TimeoutExceeded`) | none — the failure is a `Failed` tie-off; no deadline was exceeded |
| Profile/knot/loom/tie-off formats, event queue schema | — | unchanged |

**Migration: none required.**

- No profile, knot, loom, or tie-off format changes — existing rigs
  keep working as-is on 0.38.0.
- The new `ContextCompacted` loom-log variant degrades gracefully:
  older binaries reading a 0.38.0 loom-log skip the line with a
  warning (the 0.33.0 `EventsDispatched`-tuple precedent); 0.38.0
  reads old logs unchanged. No log migration.
- New rigs get `.pi/settings.json` automatically when `knot-init`
  initialises them. **Existing rigs** (already initialised) can opt
  in by creating the one-line file at the project root:
  `{ "compaction": { "enabled": true } }` — without it, 0.38.0
  behaves as before for over-full contexts.

---

### Overflow Without Compaction — Fail Fast, Warn at Startup (Knot 0.38.1, 2026-08-27)

**What changed:** 0.38.0's fail-fast only fired when pi's own
compaction had run (detected via `compaction_end` stream events). For
rigs where compaction is **disabled** (e.g. initialised before
knot-init 4.6.0 seeded `.pi/settings.json`, with the global setting
off), pi emits no compaction events at all — the overflow surfaced as
the provider's error message on the failed turn and the session-resume
nudge loop burned all 11 attempts, each re-entry making the over-full
session fuller. 0.38.1 closes the gap:

- The JSON adapter now captures the failed turn's provider error
  message and classifies it against the common context-overflow
  signatures (llama.cpp, Anthropic, OpenAI, Gemini, xAI, Groq,
  OpenRouter, Together, Copilot, LM Studio, MiniMax, Kimi, z.ai, and
  generic token-limit fallbacks; rate-limit/throttling messages
  excluded). A match with an empty response fails the strand
  immediately with `context limit reached: context overflow, but pi
  auto-compaction did not run (…provider error…). Enable compaction
  for rig sessions with a project-level .pi/settings.json: …` — no
  session-resume retries. Non-overflow errors (e.g. rate limits) keep
  the existing nudge-loop behaviour.
- At startup (service and `knot step`), knot resolves the effective pi
  compaction setting the same way pi does (project
  `.pi/settings.json` over global `~/.pi/agent/settings.json`, pi's
  default enabled) and prints a WARNING naming the project settings
  file when it resolves to disabled.

**Why:** the 0.38.0 fix assumed compaction was enabled for every rig
session; pre-4.6.0 rigs were not, and the failure mode regressed to
exactly the 11-attempt clock-up 0.38.0 set out to stop (observed in
the `borrow-my-stuff` rig). The startup warning makes the gap visible
before a strand hits the wall.

**Affected documents:** none — no project document (profile, knot,
loom, tie-off) format change.

| Artifact | Before 0.38.1 | 0.38.1+ |
|---|---|---|
| Context overflow with compaction disabled in pi settings | empty response → nudge loop re-enters the over-full session; all 11 attempts burn → `no final response: … (session resume exhausted)` | immediate `context limit reached: …` failure naming the provider error and the `.pi/settings.json` fix; no retries |
| Context overflow with compaction enabled | unchanged (0.38.0 `compaction_end`-based detection) | unchanged |
| Non-overflow provider error (rate limit, 5xx) | unchanged (nudge loop retries) | unchanged |
| Startup when compaction resolves to disabled | silent | WARNING with the project settings file path |
| Profile/knot/loom/tie-off formats, event queue schema | — | unchanged |

**Migration: none required.**

- No document format changes — existing rigs keep working as-is.
- Rigs where compaction is still disabled (no project
  `.pi/settings.json`) now fail overflows fast and clearly instead of
  burning retries; to get in-process recovery, create the one-line
  file at the project root: `{ "compaction": { "enabled": true } }`
  (or re-run `knot-init` — it seeds the file create-if-absent).

---

### Queue Entry Identity Self-Heal — Filename Is the Event ID (Knot 0.37.0, 2026-08-25)

**What changed:** the filename stem of a queued event file
(`tie-offs/<rig>/events/{id}.json`) is now the queue entry's
identity, and the queue self-heals when a file's JSON `id` drifts
from its name. On every scan, a file whose JSON `id` differs from
its filename stem is repaired in place — atomically rewritten
(temp → rename) with `id := stem` — and one warning is logged to
the service log per repaired file:
`[queue] repaired event file {name}: id {old} -> {stem} (filename
is the queue identity)`. `queued_at` and all other fields are
preserved. The repair is idempotent: after the first scan that
touches the file, name == id and no further rewrites occur.
Because FIFO order is filename sort, **renaming a queued event's
file reorders the FIFO** — that is now the supported way to front
a queued event (e.g. a manual rectify). A head that vanishes
between scan and read (concurrent removal) is logged with the file
name and handled gracefully — no silent wedge, no panic. No
document or format change.

**Why:** before 0.37.0, FIFO order came from the filename sort
while every file operation (read, dedup, late removal, restart
reload) resolved paths from the JSON `id`. A renamed queue file
(filename ≠ JSON id) therefore made the head unresolvable:
`front()` swallowed the read failure and returned `None`, the
consumer loop read a non-empty queue as empty and went permanently
idle with no log line, and the queue's own bookkeeping amplified
the divergence — a restart re-wrote the event under its original
id, duplicating it, and late removal deleted the restored file,
orphaning the renamed one. One externally touched file could wedge
a running rig until the file was deleted by hand (2026-08-25
incident: two permanent idles, each cleared only by a process
restart).

**Affected documents:** none — no project document (profile, knot,
loom) changes; the queue file schema is unchanged.

| Artifact | Before 0.37.0 | 0.37.0+ |
|---|---|---|
| Queue entry identity | JSON `id` (filename was FIFO sort order only) | filename stem (a drifted `id` is repaired to the stem on scan) |
| Renamed queue file (filename ≠ `id`) | phantom head: `front()` silent `None` (permanent idle), `pop()` panic | repaired in place on first scan (one warning per file); the rename keeps its FIFO position |
| Head vanishing between scan and read | silent `None` / panic | warning naming the file; graceful `None` (rescan-and-retry for `pop()`) |
| Document formats, queue file schema (`tie-offs/<rig>/events/*.json`) | — | unchanged |

**Migration: none required.**

- No document format changes — profiles, knots, looms, and tie-offs
  are untouched, and the queue file schema is unchanged, so a queue
  persisted by an older version is picked up as-is on the first run
  of 0.37.0. No queue migration, no file edits.
- Queues containing renamed or hand-edited event files (filename ≠
  JSON `id`) **self-heal on the first scan** of the new binary: the
  JSON `id` is rewritten to the filename stem (atomic temp → rename)
  with one stderr warning per repaired file, and the operator's FIFO
  position is kept. No file edits needed.
- Renaming a queue file to reorder the FIFO (e.g. fronting a manual
  rectify event) is a **supported operation** from 0.37.0 — the
  queue repairs the internal id on the next scan and `queued_at`
  remains the honest record of when the event was queued.

---

### Persistent Queue Wake — No Lost Queue Notifications (Knot 0.37.0, 2026-08-25)

**What changed:** internal change to how the event-queue consumer
waits for work — no document or format change. The queue wake is now
persistent by construction: `StrandEventQueue::notified()` registers
its `tokio::sync::Notify` permit **at call time** (armed, not lazy),
and both consumer loops — the service loop and `knot step`'s head
wait — **arm the wait before re-checking `front()`**. A push that
lands before the arm is visible to the fresh `front()` disk scan; a
push that lands after the arm is captured by the armed permit. The
wake guarantee is now explicit in Knot's own code, pinned by unit
tests, and independent of tokio version details.

**Why:** previously the guarantee leaned on a tokio implementation
detail — the pinned tokio 1.52.3 already stores a `notify_one` permit
when no waiter is registered — so no wake was being lost in the
field. This change removes that latent dependency rather than fixing
a live bug: the wait is correct by construction on any tokio version.

**Wake guarantee:** a queued event always wakes the processor; the
only empty-queue state is a genuinely empty `events/` directory.

**Affected documents:** none — no project document (profile, knot,
loom) changes.

| Artifact | Before 0.37.0 | 0.37.0+ |
|---|---|---|
| `StrandEventQueue::notified()` | permit registered on first poll (lazy) | permit registered at call time (armed) |
| Service loop idle wait | `front()` check, then arm + wait | arm, then `front()` check, then await the armed future |
| `knot step` head wait | `front()` check, then arm + deadline wait | arm, then `front()` check, then deadline-wait the armed future |
| Document formats, queue file schema (`tie-offs/<rig>/events/*.json`) | — | unchanged |

**Migration: none required.**

- No document format changes — profiles, knots, looms, and tie-offs
  are untouched, and the queue file schema is unchanged, so a queue
  persisted by an older version is picked up as-is on the first run
  of 0.37.0. No queue migration, no file edits.
- The behaviour change is internal (queue wake reliability). Nothing
  to migrate; the guarantee — a queued event always wakes the
  processor, and the only empty-queue state is a genuinely empty
  `events/` directory — is now explicit in the binary rather than a
  consequence of the pinned tokio version.

---

### Thinking Level — Alias Default with Profile Override (Knot 0.36.0, 2026-08-24)

**What changed:** profiles and model-registry aliases can now set a
reasoning effort — a **thinking level** — for the pi invocation. Two
new **optional** fields, both named `thinking-level`, with allowed
values `off | minimal | low | medium | high | xhigh` (anything else is
a parse-time rejection):

1. **`rig/models.yml` alias default** — an alias entry may carry a
   `thinking-level`; it is the default reasoning effort for every
   profile that resolves the alias.
2. **Profile frontmatter override** — `rig/profiles/{name}.md` may
   carry a `thinking-level`; when present it **takes precedence over
   the alias default** (the profile is more specific than the alias).

Resolution (the **effective** level): a `model-ref` profile resolves
to its own `thinking-level`, else the alias's; a direct-spec profile
(`provider` + `model`) uses its own `thinking-level` only — the
registry is not consulted, mirroring how `provider`/`model` are
sourced. The effective level is emitted as `--thinking <level>` on
the pi CLI (placed with the model options, after `--model`).

**`off` vs omission — the asymmetry is intentional:** an explicit
`off` emits `--thinking off`, *forcing* off and overriding pi's
settings default. **Omitting** the field emits no flag at all — pi's
own settings default applies. Absence is **not** `off`.

Error behaviour matches each file's existing conventions: an invalid
registry value is a warning and the whole registry is treated as
empty (never blocks processing); an invalid profile value is a hard
profile parse error (`InvalidThinkingLevel`).

| Artifact | Before 0.36.0 | 0.36.0+ |
|---|---|---|
| `rig/models.yml` alias entry | `provider`, `model` | + optional `thinking-level` |
| Profile frontmatter | `model-ref`/`provider`/`model`, `tools`, `timeout` | + optional `thinking-level` |
| pi invocation argv | `-p --model <model>` (+ `--tools` …) | + `--thinking <level>` (only when an effective level exists) |
| `state.json` profile entry | `model-ref`, `provider`, `model`, `timeout` | + `thinking-level` (the **effective** level; key omitted when unset) |

**Migration: none required.**

- Both fields are optional — existing `models.yml` files and profile
  files parse unchanged on 0.36.0. No file edits are needed.
- `state.json` readers see the new key **only** when a level is set;
  it is omitted (never `null`) otherwise, so existing consumers are
  unaffected.
- No existing field changed meaning. Profiles and aliases without a
  `thinking-level` behave exactly as before (pi's settings default
  applies — no `--thinking` flag is emitted).

#### Adopting (optional)

1. **Set an alias default** in `rig/models.yml` for a reasoning
   model:
   ```yaml
   models:
     frontier:
       provider: anthropic
       model: claude-sonnet-4-20250514
       thinking-level: high
   ```
2. **Override per profile** where a role needs more (or less):
   ```yaml
   ---
   name: analyst
   model-ref: frontier        # alias default: high
   thinking-level: xhigh      # profile override wins
   ---
   ```
3. **Verify:** `tie-offs/<rig>/state.json` shows the effective
   `thinking-level` on the profile entry (profile override or alias
   default; absent when neither sets one).

#### If Not Migrated

Nothing breaks. Without a `thinking-level` anywhere, no `--thinking`
flag is emitted and pi's own settings default applies — exactly the
pre-0.36.0 behaviour.

#### Fields Unchanged by This Migration

Profile frontmatter (`name`, `model-ref`, `provider`, `model`,
`tools`, `timeout`), knot frontmatter (`name`, `agent-profile-ref`,
`strand-dir`, `git-versioned`, `strand-source`, `event-description`),
loom format, and tie-off format are unchanged. `rig/models.yml` and
profile frontmatter gain the optional `thinking-level` field only.

---

### knot step — Single-Event Stepping and Late Queue Removal (Knot 0.35.0, 2026-08-23)

**What changed:** two changes to how queued events are consumed — a new
CLI command and new queue-removal timing.

1. **New `knot step` command** — processes exactly **one** queued
   event, then exits, running the full service startup (migration,
   config seeding, rig git init, loom discovery, watchers, debounce
   engine, state writer) and the service-identical graceful shutdown
   cascade. `knot step [--rig <rig-name>] [--event <spec>]`:
   `--event` targets a specific queued event (exact id, `.json`
   optional; unique id prefix; or strand filename — no match or an
   ambiguous match lists the queue on stderr and exits 1); without it
   the FIFO head is processed; an empty queue prints `queue empty` and
   exits 0. Events dispatched *during* the step are captured into the
   queue but not executed. Step mode does **not** clear the
   loom-logs/rig-log (a multi-step session accumulates) — only
   service startups clear them. Step rig discovery is stricter than
   the service: zero `*-rig` matches is an error (no implicit `rig/`
   creation), multiple matches is an error.
2. **Late queue removal (at-least-once delivery).** The queued event
   file is no longer removed when the event is *popped* for
   processing. On success it is removed as the **last step before the
   git commit** — dispatch, tie-off append, loom-log entries, and
   event enforcement all happen first, and the commit captures
   everything, including the removal. On failure/skip it is removed at
   the point of failure (consume-on-failure — no poison-pill retry
   loops). Invariant: every return from event processing removes the
   file exactly once.

**Affected documents:** none — no project document (profile, knot,
loom) changes.

| Artifact | Before 0.35.0 | 0.35.0+ |
|---|---|---|
| Queued event file removed | when popped, *before* processing | after the work is done (just-before-commit on success; point of failure on failure/skip) |
| Crash mid-processing | event lost (file already gone) | event survives; restart re-queues it and the knot re-runs (safe by idempotency) |
| CLI | `knot [rig-name]`, `knot share <rig>` | + `knot step [--rig <rig>] [--event <spec>]` |
| Service loop | `pop()` (delete-on-read) | `front()` (peek) + explicit removal inside processing |

**Migration: none required.**

- No document format changes — profiles, knots, looms, and tie-offs
  are untouched.
- **Pending events from older versions read identically** — the
  `tie-offs/<rig>/events/*.json` schema is unchanged, so a queue
  persisted by 0.34.0 (or earlier) is picked up as-is on the first
  run of 0.35.0. No queue migration, no file edits.
- Workflows that assumed the event file is gone as soon as processing
  *starts* must now treat it as present until the work is *done*: during
  a long agent run the file is still in `events/` (visible in
  `state.json`'s `strand_queue`).
- `knot step` is for when the service is **not** running: two
  processes sharing the disk queue can double-read the same event (the
  knot re-run is safe by idempotency, but wasteful).

---

### Per-Run Logs — Loom-Logs and Rig-Log Cleared at Startup (Knot 0.34.0, 2026-08-23)

**What changed:** the operational logs are now per-run. On every
startup — after legacy-layout migration, before loom discovery — Knot
truncates the rig-log (`tie-offs/<rig>/.rig-log`) and **every**
`tie-offs/<rig>/<loom-id>/.loom-log` (including orphaned loom dirs
whose loom no longer exists in the rig). Each log therefore always
contains exactly the events of the current knot process run: it
starts with the fresh `KnotRegistered`/`LoomStarted` events and ends
with `LoomStopped` at shutdown. Previously the logs grew
indefinitely across runs, and stale unparseable lines re-fired a
`WARN:` skip on every 5-second state write, every query, forever.

**Affected documents:** none — no project document (profile, knot,
loom) changes.

| Artifact | Before 0.34.0 | 0.34.0+ |
|---|---|---|
| `.rig-log` | accumulated across runs | truncated at every startup (current run only) |
| `*/.loom-log` | accumulated across runs | truncated at every startup (current run only) |
| Tie-off files | unchanged | unchanged (durable audit history) |
| `state.json`, `events/`, dispatch dirs | unchanged | unchanged |

**Migration: none required.**

- No document format changes — profiles, knots, looms, and tie-offs
  are untouched.
- On the **first run of the new binary**, all `.rig-log`/`.loom-log`
  content from earlier runs is discarded. This is intentional: the
  logs' purpose is current-run observability, and the durable audit
  history lives in the git-versioned tie-offs, which are unchanged.
- Any workflow that assumed cross-run log history (e.g. counting
  events "in the last 24 hours" from a loom-log) must now count
  "since the last startup" — the log holds the current run only.
- Clearing is non-fatal: a failed clear logs a `WARNING:` and startup
  proceeds. Only files named `.loom-log` (top-level per loom dir) and
  `.rig-log` are touched — nothing else is deleted or modified.

---

### Unique Event Dispatch Filenames — Per-Batch Sequence Suffix (Knot 0.33.0, 2026-08-22)

**What changed:** when one dispatch batch (one parsed tie-off, including
event-enforcement follow-ups) fans multiple events out into the *same*
consumer directory — same event id, same consumer loom, same second —
event files now carry a per-batch sequence suffix, and event files are
created atomically. Before this version all writes targeted the single
path `event-{ts}.md` and the last write silently won (incident
2026-08-22: a four-way `ValidationFail` fan-out lost three of four
events). The same fix also covers the rarer path where two consumer
knots in one loom subscribe to the same event.

**Event file naming (`tie-offs/<rig>/<consumer-loom>/<EventId>/`):**

| Batch size | Filenames |
|---|---|
| 1 (unchanged) | `event-{ts}.md` |
| N > 1 | `event-{ts}-001.md`, `event-{ts}-002.md`, … `event-{ts}-NNN.md` |

- `{ts}` is unchanged: dispatch clock at second precision, `:`/space
  replaced by `-` (e.g. `event-2026-08-22T21-54-49+01-00.md`).
- The suffix is a 3-digit zero-padded number starting at `001` — the
  plain name stays reserved for single-dispatch batches. Suffix order
  follows the producer's emission order.
- Creation is atomic (`create_new`): if the computed name is already
  taken — a leftover from an earlier run in the same second, or a
  concurrent dispatch — the suffix is bumped until a free name is
  found (bounded retry). A silent overwrite is no longer possible.

**Migration: none required.**

- **Consumers unchanged** — event-file detection is prefix-based
  (`event-`) plus frontmatter, so the suffix is transparent. N distinct
  files in one second produce N distinct consumer strands.
- **Loom-log `EventsDispatched` entries** extend their `dispatches`
  array from 3-tuples to 4-tuples (the created file path is added).
  This is an internal runtime artifact — no project document changes.
  When the new binary reads loom-logs written by an older binary,
  legacy 3-tuple lines are skipped with a warning; the warnings
  self-settle as the logs are re-read, and no data is lost (the
  producer's append-only tie-off retains all events).

---

### Model Aliases — Rig-Level Model Registry (Knot 0.32.0, 2026-08-20)

**What changed:** profiles can now reference a rig-level model alias
instead of hard-coding `provider` + `model`. A new registry file
`rig/models.yml` maps **aliases** to `{provider, model}` pairs. The
registry is read fresh at resolution time (per strand processing, per
state write) — never cached. Swapping a model is a single edit in
`models.yml`; every profile referencing the alias picks it up live,
without a restart.

**New file: `rig/models.yml`**

```yaml
models:
  fast:
    provider: openai
    model: gpt-4o
  frontier:
    provider: anthropic
    model: claude-sonnet-4-20250514
```

- Top-level `models` map: alias → `{provider, model}`.
- Both `provider` and `model` are required, non-empty per alias.
- Alias names: any non-empty string (no slug enforcement). Two
  aliases may target the same model (A/B swapping is a feature).
- File missing, empty, or comments-only → empty registry. Malformed
  YAML, or an entry with a missing/empty `provider`/`model` → warning;
  treated as an empty registry (never blocks processing).
- `run_startup` auto-creates `rig/models.yml` (commented template)
  when missing; never overwrites an existing file.

**New profile field: `model-ref`**

```yaml
---
name: fast
model-ref: fast
tools:
  - read
---
```

Precedence:

| `model-ref` | `provider` + `model` | Behaviour |
|---|---|---|
| set | absent | Resolved via registry at processing time |
| set | set | **Alias wins** — direct values ignored, parse warning |
| absent | set | Legacy direct spec — unchanged |
| absent | absent | Parse error (no model defined) |

Unknown alias → the knot run fails with `ModelRefNotFound`; the error
(`model-ref 'X' not found in rig/models.yml`) appears in the loom-log
and in `last_error` in state.

**State schema (`tie-offs/<rig>/state.json`):** profile entries gain
`model-ref` (the alias, `null` for direct-spec profiles), and
`provider`/`model` become the **resolved** values — `null` when the
alias is unresolvable.

**Direct-spec profiles are unaffected** — no forced migration.
`provider` + `model` remain valid indefinitely.

#### Migration (optional — only if you want alias-based swapping)

1. **Collect the distinct provider/model pairs** in use:
   ```bash
   grep -rE '^(provider|model):' rig/profiles/
   ```
2. **Define an alias per distinct pair** in `rig/models.yml` (semantic
   names, e.g. `fast`, `frontier`):
   ```yaml
   models:
     fast:
       provider: openai
       model: gpt-4o
   ```
3. **Replace the frontmatter lines** in each profile that uses that
   pair — locate them, then swap the two lines for one `model-ref`:
   ```bash
   grep -rl '^model: gpt-4o$' rig/profiles/
   ```
   ```yaml
   ---
   name: fast
   model-ref: fast
   ---
   ```
4. **Verify:** `tie-offs/<rig>/state.json` should show `model-ref`
   plus the resolved `provider`/`model` for each migrated profile.

#### If Not Migrated

Nothing breaks. Direct-spec profiles behave exactly as before. A
`model-ref` profile simply fails to resolve (`ModelRefNotFound`) until
its alias exists in `rig/models.yml`.

#### Fields Unchanged by This Migration

Knot frontmatter (`name`, `agent-profile-ref`, `strand-dir`,
`git-versioned`, `strand-source`, `event-description`), loom format, and
tie-off format are unchanged. Profile frontmatter gains the optional
`model-ref` field only.

---

### Rig/Project Repository Split — Runtime Tree Moves Out of the Rig (Knot 0.31.0, 2026-08-17)

**What changed:** the rig no longer holds any runtime data. The runtime
tree — tie-off directories, loom-logs, the event queue, the rig-log,
and the state snapshot — moves from `rig/` to `tie-offs/<rig-basename>/`
in the project root (default rig: `tie-offs/rig/`). The rig directory is
now **source-only** (looms, knots, profiles, config) and is versioned in
**its own git repository** (`rig/.git`), committed manually by the user.
Project commits (including Knot's per-knot-run commits) touch the runtime
tree but never the rig.

| Path | Before | After |
|---|---|---|
| State snapshot | `rig/state.json` | `tie-offs/<rig>/state.json` |
| Tie-off files | `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md` | `tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md` |
| Loom-log | `rig/tie-offs/{loom-id}/.loom-log` | `tie-offs/<rig>/{loom-id}/.loom-log` |
| Event dispatch dirs | `rig/tie-offs/{loom-id}/{EventId}/` | `tie-offs/<rig>/{loom-id}/{EventId}/` |
| Event queue | `rig/events/` | `tie-offs/<rig>/events/` |
| Rig-log | `rig/.rig-log` | `tie-offs/<rig>/.rig-log` |

**Document format:** no frontmatter changes. Profiles, knots, and looms
are unaffected — no file edits required. `last_tie_off_path` values in
state change automatically (derived at runtime); `rig_path` still points
at the rig (source) directory.

**Auto-migration (done by Knot, not the agent):** on first startup with
0.31.0, Knot moves the legacy runtime files:
`rig/tie-offs/` → `tie-offs/<rig>/` (subtree preserved),
`rig/state.json`, `rig/.rig-log`, and `rig/events/` → `tie-offs/<rig>/`.
It logs a one-line `[startup] migrated …` notice. Migration is
idempotent; if a destination already exists, the destination is kept and
a warning is logged.

**Rig repository (done by Knot, committed by the user):**

- Knot initialises `rig/.git` on startup (idempotent) and appends a
  marked `rig/` line to the project's `.gitignore` when the project root
  is inside a git repo.
- The user commits the rig git manually: `git -C rig add -A &&
  git -C rig commit -m "…"`.

**Manual step for pre-existing projects (agent action):** if `rig/` files
were already tracked by the project's git before 0.31.0, the
`.gitignore` entry alone does not untrack them. Run the one-time:

```bash
git rm -r --cached rig/
git commit -m "Untrack rig/ — now versioned in its own repository"
```

Knot detects the tracked-rig case and logs a warning instead of doing
this itself — untracking rewrites the project's index and is a
project-history decision. The Knot binary never runs `git rm`.

**Watcher re-trigger caveat:** `notify` does not rescan existing files
when a watch starts. After migration, dispatch directories that already
contained unprocessed event files are watched at their new path but the
watcher never saw those files appear. **Touch** each unprocessed event
file (`touch tie-offs/<rig>/{loom-id}/{EventId}/event-*.md`) after a
post-migration restart to re-trigger processing.

**Verification:**

```bash
# Runtime tree exists and is fresh
cat tie-offs/rig/state.json | python3 -m json.tool

# Rig is source-only
ls rig/   # looms, profiles, config — no tie-offs/, state.json, events/

# Project git ignores the rig
git check-ignore -v rig/

# Rig git exists (user commits it manually)
git -C rig status
```

---

### Init Appends Knot Terminology to AGENTS.md (skill version 1.7.0, 2026-08-10)

The `knot-init` skill (step 4b) now appends a concise **Knot
Terminology** section to the project's `AGENTS.md` when initialising a
rig. It defines the six basic Knot terms — `rig`, `loom`, `knot`,
`strand`, `tie-off`, and `event` — so the terminology is always
available during any agent session (AGENTS.md is inspected for every
task).

**Why:** Agents explore the repository and encounter knot terminology
in rig files, tie-offs, event blocks, and loom-logs regardless of how
prompts are worded. Keeping the six basic terms in AGENTS.md ensures
they are always defined, without carrying the full glossary into the
project.

**Action required for existing rigs:** if the project's `AGENTS.md` was
created before this change and does not yet contain a `## Knot
Terminology` section, append it:

```markdown
## Knot Terminology

Basic Knot terms used throughout the rig:

- **rig** — the top-level container holding looms, profiles, and rig state
- **loom** — a domain work area (a directory ending in `-loom`) grouping related knots
- **knot** — a configured task/agent workflow that processes input strands
- **strand** — a file in a knot's strand-dir that triggers the knot to process it
- **tie-off** — a knot's final output document, stored under `rig/tie-offs/`
- **event** — a message a producer knot emits for consumer knots to process

Knot terminology is encouraged inside rig files. Keep this
terminology out of skill documents and project-space documents.
```

Alternatively, re-run `knot-init`, which appends the section
idempotently.

**Affected documents:** `AGENTS.md` (additive — no frontmatter or body
semantics change). The complete glossary remains in the `knot-init`
skill at `knot-glossary.md`.

---

### Glossary — Tie-Off Definition Updated (skill version 1.6.0, 2026-08-10)

The `Tie-off` entry in the Knot glossary has been reworded to clarify
that the agent never needs to manually write to the tie-off directory.
Knot automatically captures the agent's output and stores it.

**No migration required** — the glossary is a reference document read by
agents at invocation time. No project documents or frontmatter fields
are affected.

**Action required:** the glossary lives inside the `knot-init` skill at
`knot-glossary.md`. After updating, copy it to the global skill location
so agents on other projects see the change:

```bash
cp .agents/skills/knot-init/knot-glossary.md \
   ~/.agents/skills-library/knot-init/knot-glossary.md
```

**Affected documents:** none — glossary text only.

---

### 0.30.1 — EventsDispatched Dispatches Tuple Expanded (2026-07-24)

The `EventsDispatched` loom-log event now records three-tuples in the
`dispatches` array instead of two-tuples.

**Why:** Previously the `dispatches` array was `(event-id, loom-id)`.
When two knots in the same loom both subscribe to the same event,
the duplicate was hard to debug. Now it is
`(event-id, consumer-knot-id, consumer-loom-id)` so the consumer knot
responsible for each dispatch is visible.

This affects the `.loom-log` JSONL files written to
`rig/tie-offs/{loom-id}/.loom-log`. It is an **internal runtime
artifact** — no user-authored documents are affected.

#### Affected Files

| What Changed | Old Format | New Format |
|---|---|---|
| `EventsDispatched.dispatches` | `[["EventId","loom-id"]]` | `[["EventId","consumer-knot-id","loom-id"]]` |

#### Migration

The loom-log reader already skips unparseable lines with a warning:

```
WARN: loom-log {loom-id} line {N}: skipping non-JSONL content: invalid length 2, expected a tuple of size 3
```

These old entries are harmless noise — Knot continues reading the
remaining lines. To eliminate the warnings, either:

1. **Delete old events** — truncate the `.loom-log` file to remove
   pre-migration entries:
   ```bash
   # Find the first line written by the new binary (has 3-element tuples)
   grep -n 'EventsDispatched.*"dispatches":\[\[' rig/tie-offs/{loom-id}/.loom-log
   # Check which lines have 2 vs 3 elements, then keep only new entries
   ```

2. **Let it settle** — old entries stay in the log, warnings appear
   on each state read. They do not affect processing. As the log
   grows, the noise becomes proportionally smaller.

#### If Not Migrated

- Old `EventsDispatched` lines are skipped with a warning on every
  loom-log read (state write, activity endpoint, knot status endpoint)
- No data loss — the skipped line is only an audit trail entry
- New processing events produce correct 3-tuple entries

#### Fields Unchanged by This Migration

All profile and knot frontmatter fields are unchanged. Only the
internal loom-log `EventsDispatched` event schema is affected.

---

### Guidance — Terminology Rule Relaxed (skill version 1.4.0)

The `knot-design` skill relaxes its "Never Leak Internal
Terminology" rule from a strict prohibition to a guideline.
Knot-specific terms (`strand`, `tie-off`, `knot`, `loom`, `event`)
are **encouraged** in knot body instructions, profiles,
`event-description` fields, and tie-offs — because every agent
invocation includes `AGENTS.md` referencing the Knot glossary, these
terms are always defined.

**No migration required** — the rule was universally violated in
practice (all 24 knot files and 12 event descriptions already used
knot terminology). Relaxing the rule makes existing practice compliant.

**Scope boundary preserved:** knot terminology remains **forbidden**
in skill documents (`.agents/skills/*`) and project-space documents
(`project/`). These remain orchestrator-agnostic per the
`knot-abstractions` layering.

**Affected documents:** none — no frontmatter fields or body
semantics change. Only authoring guidance changes.

---

### 0.26.0 — Tie-Off Filenames Renamed (2026-07-13)

Tie-off output files renamed from `{knot-name}-tie-off.md` to
`tie-off-{knot-name}.md`.

**Why:** The `tie-off-` prefix groups tie-off files together in
`rig/tie-offs/{loom-id}/`, making them visually distinct from event
subdirectories and other files that may appear in the same directory.

#### Affected Files

| What Changed | Old Filename | New Filename |
|---|---|---|
| Tie-off files | `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md` | `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md` |

#### Migration Steps

Tie-off files are append-mode history. Existing files at the old name
are harmless — Knot will create new files with the renamed path on the
next processing event. To migrate existing history:

1. **Find all tie-off files:**
   ```bash
   find rig/tie-offs/ -type f -name '*-tie-off.md'
   ```

2. **Rename each file** — move the knot name from prefix to suffix:
   ```bash
   for file in rig/tie-offs/*-loom/*-tie-off.md; do
     name="${file##*/}"                          # filename with extension
     name="${name%-tie-off.md}"                   # strip suffix → knot name
     dir="${file%/*}"                             # directory
     mv "$file" "${dir}/tie-off-${name}.md" 2>/dev/null
   done
   ```

   Or use a find+rename one-liner:
   ```bash
   find rig/tie-offs/ -type f -name '*-tie-off.md' | while read file; do
     base="${file##*/}"                          # filename with extension
     base="${base%-tie-off.md}"                   # strip suffix → knot name
     dir="${file%/*}"                             # directory
     mv "$file" "${dir}/tie-off-${base}.md" 2>/dev/null
   done
   ```

3. **Verify** — Knot must be running:
   ```bash
   cat rig/state.json | python3 -m json.tool
   ```
   Check that knot `last_tie_off_path` values use the new `tie-off-{knot-name}.md` format.

#### If Not Migrated

- Old-named files remain on disk (harmless orphan files)
- New processing events create tie-off files at the new filename
- No data loss — append-mode means existing content in old files is preserved
- Knot's `state.json` will reference the new filenames going forward

#### Fields Unchanged by This Migration

Only the filesystem filename of tie-off output files is affected.
All profile and knot frontmatter fields, knot definitions, and
loom definitions are unchanged.

---

### 0.24.0 — StrandSource: Unified Input Direction (2026-07-10)

The `listens-for` array is replaced by `strand-source` (expressed as
`strand-dir` with an `event:` URI). Each knot now has exactly **one**
input direction, restoring the "one strand, one direction" principle.
Event consumer knots set `strand-dir` to an `event:` URI instead of
a filesystem path, and an optional `event-description` field provides
the semantic contract injected into the producer's prompt.

#### Affected Documents

| Document Type | Location | Change |
|---------------|----------|--------|
| Knot | `rig/*-loom/*.md` | `listens-for` removed; `strand-dir` now accepts `event:` URIs; new optional `event-description` field |

#### Frontmatter Changes

**Before (dual input — filesystem path + event intents):**

```markdown
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "project/reviews"
listens-for:
  - target-knot: quality-reviewer
    event-id: ReviewCompleted
    event-description: >
      Emitted when a quality review is complete.
---

Create a refactor plan when a quality review is complete.
```

**After (single input — event URI replaces listens-for):**

```markdown
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "event:quality-reviewer:ReviewCompleted"
event-description: >
  Emitted when a quality review is complete.
---

Create a refactor plan when a quality review is complete.
```

**Normal (non-event) knots are unchanged:**

```markdown
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
---

Review the goals section.
```

#### Migration Steps

1. **Find knots with `listens-for`:**
   ```bash
   grep -rl "listens-for:" rig/ 2>/dev/null
   ```

2. **For each consumer knot with `listens-for`:**
   a. Read the knot file.
   b. For each intent in `listens-for`, take the `target-knot` value
      (the producer) and the `event-id` value.
   c. Replace `strand-dir` with:
      `"event:<target-knot>:<event-id>"`
   d. Move `event-description` from the intent object into a top-level
      `event-description` frontmatter field.
   e. Remove the entire `listens-for:` block from the frontmatter.

   **If a knot has `listens-for` with multiple intents**, it cannot
   be migrated directly — `StrandSource` supports only one input.
   Create separate knots, one per event subscription.

3. **Verify** — Knot must be running:
   ```bash
   cat rig/state.json | python3 -m json.tool
   ```
   Check that consumer knots appear without errors and that `listens-for`
   is no longer present in any knot definition.

#### If Not Migrated

- Knots with `listens-for` will parse with an **unknown-property warning**
  (`listens-for` is no longer a recognised frontmatter key).
- The knot will not have event subscriptions (the `listens-for` entries
  are silently ignored).
- The knot's `strand-dir` filesystem path still works normally.
- No data loss — existing tie-off files and event directories are
  preserved.

#### Fields Unchanged by This Migration

All other frontmatter fields keep the same meaning and location:

| Document | Field | Unchanged |
|----------|-------|-----------|
| Profile | `name` | Yes |
| Profile | `provider` | Yes |
| Profile | `model` | Yes |
| Profile | `tools` | Yes |
| Profile | `timeout` | Yes |
| Knot | `name` | Yes |
| Knot | `agent-profile-ref` | Yes |
| Knot | `strand-dir` | Yes (now also accepts `event:` URIs) |
| Knot | `git-versioned` | Yes |

---

### 0.23.0 — Intent-Based Event Routing (2026-07-09)

Intent-based event routing adds first-class agent-to-agent events.
Consumer knots declare `listens-for` intents in their frontmatter;
Knot injects event instructions into producer prompts and dispatches
matching events to consumers at runtime.

#### Affected Documents

| Document Type | Location | Change |
|---------------|----------|--------|
| Knot | `rig/*-loom/*.md` | New optional field: `listens-for` |

#### New Frontmatter Field: `listens-for`

Knots can now declare event intents using the `listens-for` YAML list:

```markdown
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "../../tie-offs/review-loom/ReviewCompleted/"
listens-for:
  - target-knot: quality-reviewer
    event-id: ReviewCompleted
    event-description: >
      Emitted when a quality review is complete.
---

Create a refactor plan when a quality review is complete.
```

Each intent entry has three fields:

| Field | Required | Description |
|-------|----------|-------------|
| `target-knot` | Yes | Which knot may emit this event (knot name) |
| `event-id` | Yes | Unique event identifier (e.g. `ReviewCompleted`, `PlanCreated`) |
| `event-description` | Yes | When the event fires and what data it should contain |

**Consumer `strand-dir`:** When using intent-based routing, the
consumer's `strand-dir` should point to the event subdirectory:
`../../tie-offs/{loom-id}/{event-id}/`. Knot creates event files
at this location when matching events are dispatched.

**Producer side:** Producers have no frontmatter changes. Knot
automatically injects event instructions into the producer's prompt
by scanning all consumers' `listens-for` declarations.

#### Migration Steps

No migration required for existing rigs. This is a new optional feature:

1. Existing knots without `listens-for` behave exactly as before.
2. To adopt intent-based routing, add `listens-for` to consumer knots.
3. Update consumer `strand-dir` to the intent event subdirectory.
4. Producers automatically receive event instructions (no config change).

#### Fields Unchanged by This Migration

All existing frontmatter fields keep the same meaning and location:

| Document | Field | Unchanged |
|----------|-------|-----------|
| Profile | `name` | Yes |
| Profile | `provider` | Yes |
| Profile | `model` | Yes |
| Profile | `tools` | Yes |
| Profile | `timeout` | Yes |
| Knot | `name` | Yes |
| Knot | `agent-profile-ref` | Yes |
| Knot | `strand-dir` | Yes |
| Knot | `git-versioned` | Yes |

---

### 0.22.0 — Tie-Off Paths Flattened (2026-07-01)

Tie-off files moved from nested knot subdirectories to flat files directly
under the loom's tie-off directory.

**Why:** The intermediate `{knot-name}` subdirectory added no value — the
tie-off filename already identifies the knot. Flattening frees the loom-level
directory for event capture subdirectories.

#### Affected Files

| What Changed | Old Path | New Path |
|---|---|---|
| Tie-off files | `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md` | `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md` |
| Loom-log | `rig/tie-offs/{loom-id}/.loom-log` | `rig/tie-offs/{loom-id}/.loom-log` (unchanged) |

#### Migration Steps

Tie-off files are append-mode history. Existing files at the old nested
location are harmless orphans — Knot will create new flat files on the next
processing event. To consolidate:

1. **Find old nested tie-off directories:**
   ```bash
   find rig/tie-offs/ -mindepth 2 -type d -name '*-tie-off.md' -prune -o -mindepth 2 -type d -print
   ```
   Or more simply, list knot-name subdirectories:
   ```bash
   find rig/tie-offs/ -mindepth 2 -maxdepth 2 -type d
   ```

2. **Move each tie-off file flat:**
   ```bash
   for knot_dir in rig/tie-offs/*-loom/*/; do
     knot_name=$(basename "$knot_dir")
     mv "$knot_dir/${knot_name}-tie-off.md" "${knot_dir%/*}/${knot_name}-tie-off.md" 2>/dev/null
     rmdir "$knot_dir" 2>/dev/null
   done
   ```

3. **Or bulk-move all at once:**
   ```bash
   find rig/tie-offs/ -mindepth 2 -maxdepth 2 -type d | while read dir; do
     name=$(basename "$dir")
     target="$dir/../${name}-tie-off.md"
     if [ -f "${dir}/${name}-tie-off.md" ]; then
       mv "${dir}/${name}-tie-off.md" "$target"
       rmdir "$dir"
     fi
   done
   ```

4. **Verify** — Knot must be running:
   ```bash
   cat rig/state.json | python3 -m json.tool
   ```
   Check that knot `last_tie_off_path` values use the flat structure.

#### If Not Migrated

- Old nested files remain on disk (harmless orphan files)
- New processing events create tie-off files at the new flat location
- No data loss — append-mode means existing content in old files is preserved
- Knot's `state.json` will reference the new flat paths

#### Fields Unchanged by This Migration

All profile and knot frontmatter fields are unchanged. Only the
filesystem location of tie-off output files is affected.

---

### 0.18.0 — Prompt text moved to markdown body (2026-06-24)

Prompt content moved from YAML frontmatter block scalars to the markdown
body (text after the closing `---`). Frontmatter now holds only structural
metadata.

**Why:** Prompt text in YAML frontmatter is indentation-sensitive, produces
noisy diffs, and inverts the normal markdown convention where the body
holds the primary content.

#### Affected Documents

| Document Type | Location | Old Field | New Location |
|---------------|----------|-----------|--------------|
| Agent Profile | `rig/profiles/*.md` | `profile-prompt: \|` in frontmatter | Markdown body after closing `---` |
| Knot | `rig/*-loom/*.md` | `prompt-template:\n  instructions: \|` in frontmatter | Markdown body after closing `---` |

#### Profile Migration

**Before:**

```markdown
---
name: fast
provider: openai
model: gpt-4o
tools:
  - read
  - bash
profile-prompt: |
  You are a fast reviewer. Keep responses concise and direct.
---

# Fast Profile

A fast reviewer profile.
```

**After:**

```markdown
---
name: fast
provider: openai
model: gpt-4o
tools:
  - read
  - bash
---

You are a fast reviewer. Keep responses concise and direct.
```

**Migration steps:**

1. Read the profile file at `rig/profiles/{name}.md`.
2. Extract the text value of `profile-prompt` (the full block scalar,
   unindented).
3. Remove the `profile-prompt` line and its block content from the
   frontmatter.
4. Replace the markdown body (everything after closing `---`) with the
   extracted prompt text. If there was a heading or summary in the
   old body, discard it — it was documentation that duplicated the
   prompt.
5. If the prompt text is long, it becomes the entire body. No heading
   wrapper needed — the body *is* the prompt.

**Search pattern:** Look for `profile-prompt: |` in any `.md` file
under `rig/profiles/`.

#### Knot Migration

**Before:**

```markdown
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
prompt-template:
  instructions: |
    Review the goals section of this PRD. Check that:
    - Each goal is specific and measurable
    - Goals align with the problem statement
---

# Goals Review

Review the goals section of this PRD.
```

**After:**

```markdown
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
---

Review the goals section of this PRD. Check that:
- Each goal is specific and measurable
- Goals align with the problem statement
```

**Migration steps:**

1. Read the knot file at `rig/{loom-id}/{knot-name}.md`.
2. Extract the text value of `prompt-template.instructions` (the full
   block scalar, unindented).
3. Remove the entire `prompt-template:` block (both `prompt-template:`
   and `  instructions: |` lines) from the frontmatter.
4. Replace the markdown body with the extracted instruction text.
   Discard any old body heading or summary — it was duplicate
   documentation.
5. If the instructions contain multiple paragraphs or lists, they
   become the body as-is (no wrapping heading).

**Search pattern:** Look for `prompt-template:` followed by
`  instructions: |` in any `.md` file under `rig/`.

#### Fields Unchanged by This Migration

These frontmatter fields keep the same meaning and location:

| Document | Field | Unchanged |
|----------|-------|-----------|
| Profile | `name` | Yes |
| Profile | `provider` | Yes |
| Profile | `model` | Yes |
| Profile | `tools` | Yes |
| Profile | `timeout` | Yes |
| Knot | `name` | Yes |
| Knot | `agent-profile-ref` | Yes |
| Knot | `strand-dir` | Yes |
| Knot | `git-versioned` | Yes |

---

## Agent Workflow

When a project updates Knot:

1. **Read this skill file** to see the full changelog.
2. **Determine the project's current Knot version** — check any
   `Cargo.lock`, `Cargo.toml`, or project notes for the previous
   version.
3. **For each changelog entry newer than the current version:**
   a. Read the migration instructions for that entry.
   b. Find affected files using the search patterns documented in the
      entry.
   c. Apply the transformations (edit frontmatter, move content to body).
   d. Verify the files parse correctly by restarting Knot and checking
      `rig/state.json` for errors.
4. **Report migration results** — list each migrated file and confirm
   Knot is reading them without errors.

---

## Adding New Changelog Entries

When Knot introduces a new format change:

1. Add a new versioned entry at the **top** of the Changelog section
   (newest first).
2. Include:
   - Version number and short description as an `###` heading
   - "Why" rationale in one paragraph
   - "Affected Documents" table mapping old fields to new locations
   - Migration steps for each affected document type (before/after
     examples + numbered steps + search patterns)
   - "Fields Unchanged" table to confirm what stays the same
3. Bump the skill `version` in the frontmatter metadata.
4. Publish the updated skill to the production library:
   ```bash
   mkdir -p ~/.agents/skills-library/knot-update
   cp -r .agents/skills/knot-update/. ~/.agents/skills-library/knot-update/
   ```

---

## Quick Reference

```bash
# Find profiles using old format (profile-prompt in frontmatter)
grep -rl "profile-prompt:" rig/profiles/ 2>/dev/null

# Find knots using old format (prompt-template in frontmatter)
grep -rl "prompt-template:" rig/ 2>/dev/null

# Publish updated skill to the production library
mkdir -p ~/.agents/skills-library/knot-update
cp -r .agents/skills/knot-update/. ~/.agents/skills-library/knot-update/

# Verify Knot is reading migrated files
cat rig/state.json | python3 -m json.tool
```

---

## Cross-Reference

Related skills:

1. **knot-create skill** — create and modify looms, knots, and profiles
2. **knot-inspect skill** — inspect rig state after migration
3. **knot-init skill** — initialise a new rig (no migration needed)

This skill records **what changed** between Knot versions. The other
skills define **how to work with** the current Knot format.
