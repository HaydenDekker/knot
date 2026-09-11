# Concepts

Knot is a file-first agent orchestration system. It watches directories for
file changes, and triggers AI agent sessions in response. The entire workflow,
the rig, is stored as plain text on disk, making it easy to review, share,
and version-control.

This page explains Knot's mental model — the hierarchy of objects and
how they relate to each other.

## The Hierarchy

```
Rig (reusable source — its own git repo)
 ├── Profiles (shared agent configurations)
 └── Looms (processing namespaces)
      └── Knots (individual processing tasks)
           ├── reads from a Strand Directory
           └── writes a Tie-off

Runtime tree (project output — committed with project git)
tie-offs/<rig>/
 ├── state.json (live observability, written on state change)
 ├── knot-service.log (consolidated service log, appended by the launcher)
 ├── events/ (disk-backed event queue)
 └── {loom-id}/ (tie-off files, event dispatch dirs)
```

### Rig

The top-level container. A rig lives at `./rig/` in your project and
aggregates all looms and profiles — **reusable source only**. All
processing output and runtime data lives in the rig's **runtime tree**
at `tie-offs/<rig-basename>/` in the project root (default rig:
`tie-offs/rig/`). The rig is versioned in its own git repository
(committed manually by the user); the runtime tree is committed with
the project's git history.

It is the ship's complete interconnected system — the place where looms
live and knots are defined.

Knot supports **rig switching** — multiple rigs in the same project
(directory names like `myproject-rig/`). Run `knot <rig-name>` to
target a specific rig, or just `knot` to auto-discover.

### Loom

A directory inside the rig whose name ends with `-loom` (e.g.
`rig/planning-loom/`). A loom is a **namespace for a domain of
responsibility** — it groups knots that work on the same kind of output.
For example, a `planning-loom` contains knots that produce or maintain
project plans.

Knot discovers looms automatically — any subdirectory of `rig/` ending
in `-loom` is registered.

### Knot

A `.md` file with YAML frontmatter inside a loom directory. A knot
brings everything together for a single processing task:

1. **Agent Profile** — which agent runs (provider, model, tools, system
   prompt).
2. **Markdown body** — task-specific instructions (the prompt text).
3. **Strand Directory** — which directory to watch for input files.

One loom can contain multiple knot files, each defining a different
processing task.

### Strand

A **text file** in a knot's strand directory. Any text file is accepted
(`.md`, `.rs`, `.json`, `.py`, `.txt`, etc.) — not just Markdown.
Binary files are detected and silently skipped (logged as
`StrandIgnored` in the service log).

When a strand is created, modified, or deleted, the knot that watches
that directory is triggered to process it. The strand is the raw input
fed into the knot's agent session.

### Tie-off

The output produced by a knot after processing. Each processing event is
appended to a single `tie-off-{knot-name}.md` file at
`tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md` (in the runtime
tree). The file grows over time, telling the complete story of the
knot's work. Event metadata in each section identifies which strand was
processed.

### Strand Directory

The directory a knot watches for strand events, configured as `strand-dir`
in the knot's YAML frontmatter. It is resolved relative to the project
root (the directory containing `rig/`).

### Rig State

`tie-offs/<rig>/state.json` is written **when the state actually changes** (the writer ticks every 5 seconds but skips no-op writes — an unchanged mtime means the rig is idle) and contains the
complete live state of the rig: registered looms, their knots with processing
status, agent profiles, and the pending strand queue. This is Knot's
primary observability interface — no HTTP API is used.

The **strand queue** (`strand_queue` array) shows all pending strand
events with file path, loom/knot IDs, event type, and queued timestamp.

## The Processing Flow

```
File change in strand-dir
        │
        ▼
  Knot's file watcher detects event
        │
        ▼
  Event debounced (avoids partial writes)
        │
        ▼
  Knot loads its agent profile from disk
        │
        ▼
  Agent session starts:
    ├── prompt = profile body + knot body + trigger line
    ├── input = strand file(s)
    └── tools = profile.tools
        │
        ▼
  Agent produces output
        │
        ▼
  Output appended to tie-off file
        │
        ▼
  Git commit created (if git-versioned: true)
```

### Session Resume

If an agent invocation fails (timeout, network error, process crash)
— or ends its turn abruptly without a final response — and a session ID
was captured, Knot automatically re-enters the same session using
`--session-id`, up to 10 retries with 10-second delays between attempts.
Each retry re-sends the original prompt with the final-response request
appended — *“Please produce your final response, or continue if you have
not finished.”* — one nudge for all resumes: “continue if you have not
finished” covers the mid-stream case, “produce your final response”
covers the abrupt-stop case. The profile's overall timeout budget is
respected — retries stop when insufficient time remains. A successful
resume completes the strand transparently, as if the first attempt had
succeeded.

An abrupt turn-end — the agent exits cleanly (exit 0) but produces no
final response — is a **failure**, not a timeout: Knot re-enters the
session to request the final response (above); only when the nudges are
exhausted does the knot end with status `failed` and
`no final response: … after N attempts (session resume exhausted)` as
the error. A failed tie-off section is written; no operational event is
recorded (only deadline breaches are — `TimeoutExceeded` records genuine
deadline breaches only).
Without a session ID (stdio adapter, unparseable output) there is no
re-entry — the terminal failure stands after the first attempt.

A third re-entry cause is **inactivity** (Knot 0.40.0+): a watchdog
kills the session when it produces *no output at all* — no thinking,
no response, no streamed tool output — for
`inactivity-timeout-seconds` (rig-global, default **300**, `0`
disables; loaded at startup, restart Knot after editing). It is a
second timer, orthogonal to the profile's overall timeout budget:
**silence is bounded by inactivity, work is bounded by the budget**.
A healthy long-running command keeps streaming output (pi relays
tool output as a throttled stream) and keeps resetting the timer, so
an active session may run past the inactivity window until the
budget; a hung command or stalled provider goes silent and is killed
at the window. Each kill is recorded as an `AgentInactivity` event in the
service log
entry — attempt, silent seconds, window, captured session ID, and the
**blocked call** named from the stream when derivable (e.g.
`bash("npm run build")`) — and the retry re-enters the same session
(fresh, when no session ID was captured) with a cause-specific note
appended to the prompt: *“Your last call blocked for more than N
seconds with no output…”* — telling the agent how to keep the session
alive (run the task in the background and poll its output, or stream
the output) instead of re-hanging. When the inactivity attempts are
exhausted the knot terminates with
`inactivity: session resume exhausted 10 retries after N inactivity
kills` — a `TimeoutExceeded` operational event (a deadline did fire) and
no tie-off write, the same shape as a total timeout.

### Context Compaction (Compact-and-Continue)

A session whose context approaches the model's context window is not a
failure: pi **compacts the session in-process and continues**.

- **Auto-compaction is always on.** Knot self-heals the project's
  `.pi/settings.json` at startup (Knot 0.45.0+): when the effective pi
  setting resolves to disabled (the global `~/.pi/agent/settings.json`
  says `compaction.enabled: false` with no project override — the shape
  of rigs initialised before `knot-init` seeded the project file), Knot
  **merges** `{"compaction": {"enabled": true}}` into the project file,
  preserving every existing key. The project file is **Knot's own file**
  (knot-init seeds it; the service maintains it); the global file is the
  operator's and is **never written**. An **explicit** project-level
  `compaction.enabled` (true **or** false) is the operator's decision
  and is honoured — an explicit `false` still raises the startup warning.
  Unparseable project files and write failures are left untouched, with
  the same warning naming the reason.
- **Proactive** — pi compacts before the hard limit
  (`contextTokens > contextWindow − reserveTokens`; 16k reserved by
default).
- **Reactive** — when the model rejects an over-full context, pi
  compacts and auto-retries the prompt in-process. Recovery is once
  per user message, so every session-resume re-entry gets a fresh
  recovery chance.
- **A compaction is a span** — pi's stream emits `compaction_start` when
  a compaction begins and `compaction_end` when it finishes. Knot
  observes both **live** (as the stream produces them) and records the
  span boundaries as loom events:
  - `CompactionStarted` — the span began: `reason` (pi's compaction
    reason: `"threshold"` / `"overflow"` / `"manual"`), `session_id`,
    `attempt`.
  - `ContextCompacted` — the span **ended successfully**. This is the
    operator-facing context-pressure signal (shape unchanged by Knot
    0.45.0 — only the timing moved from after-the-invocation to
    live): `reason` (`"overflow"` = the context limit was hit — the
    entries to count when narrowing prompt scope; `"threshold"` =
    proactive), `tokens_before` (the pre-compaction size),
    `session_id`, and `attempt` (1 = first attempt, 2 = first retry).
  - `ContextCompactionFailed` — the span ended **without success**
    (pi reported an `errorMessage`, or the compaction was aborted): the
    same fields plus `error` (pi's message, `None` when aborted
    without one) and `aborted`. Previously failed compactions were
    silent; they are now visible.
- **Continuity across re-entries** — the session id is carried from the
  runner: when a final-response nudge or a session-resume retry
  re-enters a session, it re-enters the **runner-captured** session id
  (from the invocation's stream), so a compaction observed mid-run
  never loses the session identity for the retry.
- **Terminal overflow fails fast** — if the kept context itself
  cannot fit the window even after compaction, the strand fails
  immediately with `context limit reached: …` — no session-resume
  retries, no clock-up, and no timeout operational event (no deadline was
  exceeded).
- **Interrupted compaction is recovered, not fatal** (Knot 0.46.0+) — a
  different overflow shape is *resumable*, not terminal: the in-process
  overflow compaction **started but never reported completion**
  (`compaction_start { reason: overflow }` with no `compaction_end`, and no
  terminal `errorMessage`) — i.e. pi's overflow recovery died *mid-turn*
  (process killed, inactivity, or a stream gap), leaving the context still
  over-full. This is classified as `CompactionInterrupted` (resumable: it
  carries the live session id) rather than the terminal `ContextLimitReached`.
  Knot's remedy is an **out-of-band manual compact**: it re-opens the *same*
  session with `--session <id>` and sends a `compact` RPC (with a fixed
  operator note as `customInstructions`). Because a manual compact is a
  first-class command that *always* emits a `compaction_end`, the interrupted
  span is closed and the context is actually shrunk. On success Knot records
  `ManualCompactionSucceeded` + `SessionRestarted` and re-enters the session
  with a restart note ("your context was just compacted — continue from the
  compacted state"); if the explicit compact itself cannot reduce the
  context it records `ManualCompactionFailed` and the run is terminal (a
  re-entry would overflow again). The manual compact is bounded to **one** per
  failed execution (a second interruption is left to the normal retry
  machinery).
- **The escape hatch** — `compaction.enabled` in `.pi/settings.json`
  (`true`/`false`) remains the only switch: pi has no
  "only-on-overflow" mode upstream, so the only way to turn compaction
  off is an explicit project-level `false`.

### Graceful Completion (Wrap-Up Steering)

Compaction (above) is **reactive** — it salvages the session but does
not ask the agent to stop and hand off. **Wrap-up steering** is the
**proactive** complement: with the opt-in `pi-rpc` adapter and a
per-alias `ctx-wrap-up-limit` in `rig/models.yml`, Knot watches the
session's live context usage and, once the sampled tokens cross the
limit, sends a one-shot `steer` at the next turn boundary telling the
agent to stop starting new work, commit all complete work, update its
progress, note what is incomplete and where it left off, and produce its
final tie-off. So context exhaustion ends in a clean handoff instead of
an uncommitted working tree.

- **Fire-once** — at most one steer per run. It is gated on the
  `ctx-wrap-up-limit` (a `0`/absent value disables the whole feature) and
  fires regardless of whether pi's own auto-compaction is enabled (the
  steer is the remedy either way). It is meaningful only on `pi-rpc`
  runs; under another adapter the limit is ignored with a one-shot
  warning.
- **Placement** — set `ctx-wrap-up-limit` a few thousand tokens below
  the model's effective compaction point (`contextWindow −
  reserveTokens`) so the steer lands before pi would auto-compact.
- **Loom-log visibility** — each steer is recorded as a
  `ContextWrapUpSteered` loom entry: `session_id`, `context_tokens`,
  `limit`, and `attempt`. The steer prompt itself is not persisted — it
  is an in-flight instruction; the run's own tie-off and git commit are
  the reconstructed result (the filesystem, not a transcript, is the
  source of truth).

## Event Queue

Strand events wait in a **disk-backed queue** at `tie-offs/<rig>/events/`
— one JSON file per pending event (the `strand_queue` array in
[state.json](#rig-state) is the live view of the same queue). The disk
*is* the queue: pending work survives restarts, and a queued event can
be inspected or edited with standard tools before it is processed. A
queued event always wakes the processor — the only empty-queue state
is a genuinely empty `events/` directory. The filename stem is the
queue entry's identity: renaming a queued event's file reorders the
FIFO, and on its next scan the queue repairs the file's internal id
to the filename stem (`queued_at` is preserved).

### At-Least-Once Delivery (Late Removal)

The queue offers **at-least-once** delivery: a queued event file is
removed only *after* the work it represents is done, never before it
starts.

- **On success** the event file is removed as the **last step before the
  git commit** — the tie-off append, dispatch of emitted events,
  service-log entries, and event enforcement all happen first, and the
  commit captures everything, including the removal.
- **On failure or skip** the event file is removed **at the point of
  failure** (consume-on-failure — a broken event does not poison the
  queue with retry loops).
- Invariant: *every* return from event processing removes the file
  exactly once.

Because removal is late, the only window in which a queued event
survives a crash is **while its processing is in flight**. On restart
the event is re-queued and the knot re-runs — safe because knots are
[goal-seeking and idempotent](#goal-seeking-not-scripted).

### Stepping: `knot step`

`knot step` processes exactly **one** queued event and then exits —
the FIFO head by default, a specific queued event with `--event`, a
specific rig with `--rig`. It runs the full service startup, executes
the single event, captures any events dispatched *during* the step
without executing them, and shuts down gracefully. Use it to observe
one cycle at a time between events; see the `knot-dispatch` skill for
the full workflow.

## Reacting to Knot Outcomes

Every **system event** Knot writes to the loom-log / rig-log is also
dispatchable to subscriber knots — not just the events an agent chooses
to emit. A system event is a terminal or lifecycle *fact* about a run:

- **Run outcome** — `KnotProcessing`, `KnotFailed`, `KnotCompleted`,
  `KnotEventsMissing`, `TimeoutExceeded` (knot-scoped); `StrandIgnored`,
  `StrandSkipped`, `StrandProcessed` (loom-scoped).
- **Retry / session** — `SessionResumed`, `KnotEmptyResponse`,
  `AgentInactivity`, `ContextCompacted` (knot-scoped, **per attempt**).
- **Loom / knot lifecycle** — `LoomStarted`, `LoomStopped`,
  `KnotRegistered`, `KnotDeregistered`, `KnotParseWarning`,
  `DirectoryCreated`.
- **Rig lifecycle** — `QueueIdle` (rig-scoped).

Subscribe with the same `event:` `strand-dir` URI, adding two new
producer-token positions on top of knot-level and loom-level:

- **Wildcard** `event:*:<EventId>` — any knot in the rig (e.g. a
  `event:*:KnotFailed` monitor that reacts to any failure).
- **Rig-level** `event:<rig-id>:<EventId>` — a rig-scoped event
  (e.g. `event:<rig>:QueueIdle` when the queue drains after a burst).

Two rules to keep reactions safe: a system event is **never** dispatched
back to the knot that produced it (so a knot's own failure does not
re-trigger it), and per-attempt events fire once *per retry attempt* —
so any failure/retry subscriber must be **idempotent**. `EventsDispatched`
is intentionally not dispatchable. See the `knot-create` skill's
**System Events** catalog and the `knot-design` skill's loop-discipline
notes for the full reference.

## Git Versioning

By default, Knot creates a git commit in the **project** repository
after each successful tie-off write. The commit message includes the
knot ID, event type, and strand filename; tie-off content forms the
commit body. The commit touches the runtime tree (`tie-offs/<rig>/`)
and any project files the knot wrote — never the rig directory
(`rig/` is excluded from project commits).

Per-knot opt-out: set `git-versioned: false` in the knot's YAML
frontmatter. If the project is not a git repo, commits are silently
skipped. The rig's own git repository is separate and committed
manually by the user.

## Logs

Knot has a single consolidated **service log** (plan 083): one line per
record on the service's stderr, timestamped and tagged:

```
[2026-09-07T15:35:57+10:00] [KNOT][EVENT] KnotCompleted loom=review-loom knot=review strand=… tie-off=…
[2026-09-07T15:36:02+10:00] [KNOT][STATE] change knot review-loom/review: status idle→completed
```

- **`[KNOT][EVENT]`** — one line per domain event: loom lifecycle
  (`LoomStarted`, `KnotRegistered`, …), strand processing
  (`KnotProcessing`, `KnotCompleted`, `KnotFailed`, `StrandProcessed`,
  …), timeouts (`TimeoutExceeded`, `AgentInactivity`), and queue idle
  (`QueueIdle`). The line carries the event's fields as `key=value` pairs
  (e.g. `loom=`, `knot=`, `strand=`, `tie-off=`, `error=`).
- **`[KNOT][STATE]`** — one line per actual `state.json` write: an
  `initial snapshot` baseline, then `change` deltas (loom/knot/profile
  additions, removals, and field changes, queue additions/drain).

Run activity is **in-memory per process** — the retired `.loom-log` /
`.rig-log` JSONL files are gone, and nothing is cleared at startup
(legacy files, if a migration moved them, are inert). The `knot-start`
skill appends the service stderr to `tie-offs/<rig>/knot-service.log`,
so that file is the durable operational record across restarts. The
tie-off files remain the durable audit record of completed work (plain
text, git-versioned), and `state.json` is the live snapshot.

Knots and operators react to events of the *current* run only (in-memory
run activity); cross-run history lives in the appended service log and
tie-offs.

## Key Principles

### File-First

All configuration lives as `.md` files with YAML frontmatter. Write files
directly to disk — Knot's file watcher picks up changes automatically.
Observation is through `tie-offs/<rig>/state.json`, written whenever the
state actually changes (idle ticks never rewrite it).

### Version-Controllable

Everything is plain text. The rig source (profiles, looms, knots) lives
in its own git repository, committed manually by the user. Runtime
output (tie-offs, logs, state) is committed to the project repository —
Knot itself creates git commits for tie-off output.

### Auto-Discovery

Knot discovers configuration automatically:

- **Looms** — any `rig/*-loom/` directory
- **Knots** — any `.md` file inside a loom directory
- **Profiles** — any `.md` file in `rig/profiles/`

Profiles are read fresh from disk at processing time — edits take effect
on the next strand event without restarting Knot.

### Goal-Seeking, Not Scripted

Knots are not one-shot scripts. They are **goal-seeking agents** that
read current state, compare it against a goal, and apply only the changes
needed. This makes them idempotent — safe to re-run on the same input.

See the [Design Guide](design-guide.md) for details on designing
idempotent knots.

## Agent Skills

Knot ships with a set of agent skills that let your AI agent manage
the rig. Each skill is a `.md` file discovered by the agent framework
(pi or others) and invoked by natural language.

| Skill | Purpose |
|-------|---------|
| **knot** | Master router — the only Knot skill auto-discovered by pi in other projects; reads the sub-skills below on demand |
| **knot-init** | Initialise a rig, create profiles, install skills globally |
| **knot-start** | Start, stop, and restart the service; append and read `tie-offs/<rig>/knot-service.log` |
| **knot-create** | Create, modify, delete looms, knots, and profiles |
| **knot-dispatch** | Trigger knots into action by creating or touching strands |
| **knot-inspect** | View rig state, looms, knots, profiles, and activity logs |
| **knot-manage** | Review completed work — tie-off quality, interaction chains, commit quality |
| **knot-analyst** | Analyse rig productivity, project progress, and blockers |
| **knot-design** | Design looms and knots following idempotency and loop patterns |
| **knot-update** | Migrate project documents between Knot versions |

Skills are developed at `.agents/skills/` in the Knot repository and
deployed to the personal skills repository (`~/.agents/`): the master
router is installed to `~/.agents/skills/knot/` (auto-discovered by
pi) and the sub-skills to `~/.agents/skills-library/` (loaded on
demand via the master's routing table). The `knot-init` skill handles
this installation automatically during rig setup.
