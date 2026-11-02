---
name: knot-dispatch
description: "Trigger knots into action by creating or touching strand files, dispatching events manually (the direct queue write), stepping the queue one event at a time with `knot step`, and understanding the event flow from producer to consumer. Covers filesystem triggers (creating/touching files in strand-dir), manual event delivery via hand-written `Event`-kind queue entries in `tie-offs/<rig>/events/`, single-event stepping (`knot step`, `--rig`, `--event`, empty-queue behaviour), verifying triggers via the service log and `tie-offs.log`, and the full dispatch pipeline from strand creation to tie-off completion."
license: MIT
metadata:
  author: Knot Team
  version: "2.1.0"
  compatibility: "Knot 0.66.0+"
---

# Knot Dispatch Skill

Trigger knots into action by adding or touching strand files, manually
queueing event deliveries, and understanding the full dispatch pipeline
from strand creation through tie-off completion.

Knots are event-driven — filesystem knots react to file changes in
their `strand-dir`, event-consumer knots react to **queue entries**
that carry the producer's emission payload. This skill covers how to
trigger them, verify they fired, and troubleshoot when they don't.

**State file:** `tie-offs/<rig>/state.json` (written when the state
changes — a background writer ticks every 5 seconds, but idle ticks
never rewrite it; default rig: `tie-offs/rig/state.json`)
**Service log:** `tie-offs/<rig>/knot-service.log` (single-line
`[KNOT][EVENT]` / `[KNOT][STATE]` records, appended across runs by
`knot-start`; filter with `grep 'loom=<id>'`) — since Knot 0.41.0 there
are no per-run log files
**Tie-off output:** `tie-offs/<rig>/tie-offs.log` — one accumulating
log, the service is the sole writer. Every knot's every run appends
one section with a structured header line (`## knot=… event=…
strand=… timestamp=… session=… source=… original-strand=…`). It is a
**log, not a state file**: query it (`grep` by knot/loom/`event-id=`),
never whole-file read it; git is the rotation

---

## Core Philosophy

### Everything Is a Strand Event

Knots have one input direction: the event queue. Filesystem knots are
triggered by file changes in their `strand-dir` (the watcher turns
them into `Created` / `Modified` / `Deleted` queue entries); event-
consumer knots — whose `strand-dir` is an `event:` URI such as
`event:quality-reviewer:ReviewCompleted` — are triggered by
`Event`-kind queue entries that **carry the producer's payload in the
entry itself**. An `event:` URI is a *subscription descriptor*: it is
not a watched path, nothing is stat-ed at it. Either way the trigger is
the same final step — a queue entry names this knot, `ProcessStrand`
picks it up. There is no "start" command — you dispatch work by
touching the filesystem or the queue.

### File Watch, Not Polling

Knot uses `notify` (inotify on Linux, FSEvents on macOS) for instant
delivery. When a file is created, modified, or deleted in a watched
directory, Knot receives the event within milliseconds. No polling loop
is needed.

### Debounce Protects Against Rapid Changes

If multiple changes occur to the same file within the debounce window
(100ms default), Knot coalesces them into a single `Modified` event.
This prevents re-processing mid-edit. The final processed state
reflects the last saved content. (Debounce applies to filesystem
entries; `Event`-kind entries queue directly, one per subscriber.)

### The Event Queue Is the Disk

Between the file watcher and the processor sits a **disk-backed event
queue**. The queue lives in `tie-offs/<rig>/events/*.json` — each
pending event is a JSON file on disk. There is no separate in-memory
event store.

```
tie-offs/<rig>/events/{timestamp}-{rand}.json
```

Entries come in **two kinds** (Knot 0.63.0+, plan 099):

- **Filesystem entries** (`kind` `Created` / `Modified` / `Deleted`)
  point at a real strand file — the entry is a *pointer*, the file
  holds the message. Right for a file trigger.
- **Event entries** (`kind` `Event`) **are the message**: they carry
  the emitted payload in a `delivery` object and no file exists to
  read. Their `strand_path` is the synthetic descriptor
  `event:<producer-knot>/<event-id>` — greppable (`grep
  'strand=event:<knot>/'`) but never a path. The consumer's prompt
  gets the rendered delivery (producer identity, event id, original
  strand, payload fields, body) prepended in place of a file it cannot
  read.

Key properties:

- **Persisted across restarts**: If Knot stops and restarts, all
  queued `.json` files are reloaded (`load_persisted`) before the
  file watcher begins emitting new events. This preserves FIFO ordering
  across restart boundaries.
- **Reorderable by rename**: FIFO order is filename sort, so renaming
  a queued event's file (e.g. to an earlier `{timestamp}-{rand}` name)
  moves it within the queue. This is the supported way to front a
  queued event (e.g. a manual rectify event). The filename is the
  queue entry's identity: on the next scan the queue repairs the
  file's internal id to the filename stem — an atomic rewrite, with
  one `[queue] repaired …` warning logged to the service log per
  repaired file. `queued_at` is preserved as the honest record of
  when the event was queued; only the internal id changes.
- **Late removal (at-least-once delivery)**: the `.json` file is
  removed only *after* the event's work is done — as the last step
  before the git commit on success, or at the point of failure on
  failure/skip. Each event file is processed exactly once, but a crash
  mid-processing leaves the file behind, so a restart re-queues it and
  the knot re-runs (safe by knot idempotency). While the agent is
  running, the event file is still in `events/`.
- **Deduplicated**: If the same file generates multiple events before
  processing (e.g. rapid saves), the older event is replaced. The
  dedup key is `(strand_path, loom_id, knot_id, event_kind)`.
- **Viewable in state**: `tie-offs/<rig>/state.json` includes a `strand_queue`
  array showing all currently pending events.

### No Dispatch Directories Anymore

Knot 0.63.0 (plan 099) retired the old per-`EventId` **dispatch
directories** (`tie-offs/<rig>/{loom-id}/{EventId}/` with dispatched
`.md` event strand files inside) and the per-knot tie-off files
(`tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md`). If an older rig
still has them, startup logs one-line `[KNOT][CONFIG]` deprecation
diagnostics naming them — the service neither reads nor backfills
them; they stay in git history. Today `tie-offs/<rig>/` holds exactly:
`state.json`, `knot-service.log`, `tie-offs.log`, and `events/`.
Producer→consumer delivery is a **direct queue write**: one `Event`-
kind entry per subscriber, payload embedded (see [Event Queue Is the
Disk](#the-event-queue-is-the-disk)). The only on-disk artefact of an
emission that persists is the producer's **git commit** (the full event
block in the body + `session:` / `event:` / `source:` /
`original_strand:` trailers) and the slim `- event-audit …` line in
`tie-offs.log`.

---

## Prerequisites

These apply to the strand-touching trigger workflow below (a running
service that watches the filesystem). The `knot step` workflow is the
opposite — it requires the service to be **stopped** (see [Stepping](#stepping-knot-step--one-event-then-stop)).

1. Knot must be running and the rig must be initialised.
   Verify by checking `tie-offs/<rig>/state.json` exists and has fresh
   `updated_at`. If not, start it with the `knot-start` skill (which
   appends `tie-offs/<rig>/knot-service.log`); use `knot-init` only when
   the rig itself does not exist yet.
2. At least one loom with at least one knot must exist.
   Check `tie-offs/<rig>/state.json` `looms` array. If empty, use
   `knot-create` to create looms and knots first.

---

## How Knots Are Triggered

### The Trigger Chain

```
File change in strand-dir
    ↓
notify fires raw filesystem event
    ↓
EventSource maps to StrandEvent (Created / Modified / Deleted)
    ↓
Debounce engine coalesces rapid changes
    ↓
Event pushed to queue (Created or Modified)
    ↓
ProcessStrand picks up the event
    ↓
Agent invoked with profile + knot instructions
    ↓
Tie-off section appended to tie-offs/<rig>/tie-offs.log (structured header line)
    ↓
Event blocks parsed → `occurred: true` events queued directly — one
payload-bearing `Event`-kind entry per subscriber (fan-out, `EventsDispatched`)
```

An event-consumer knot's trigger chain starts at that last line: its
queue entry carries the delivery, and `ProcessStrand` renders it into
the prompt.

### Three Trigger Types

| StrandEvent | When It Fires | Agent Invocation |
|-------------|---------------|-----------------|
| `Created` | New file appears in strand-dir | Yes — full prompt sent |
| `Modified` | Existing file is edited and saved | Yes — full prompt sent |
| `Deleted` | File is removed from strand-dir | Yes — deletion notice injected (no `@file` arg) |

For `Deleted` events, the agent does not receive the file content
(it no longer exists). Instead, the prompt includes a deletion notice,
git history hint, and any previous tie-off entries for that strand.

---

## Agent Workflow

### Trigger a Normal Knot (Filesystem strand-dir)

When asked to trigger, fire, or dispatch a knot that reads from a
filesystem directory:

1. **Identify the strand directory**: Read `tie-offs/<rig>/state.json`
   and find the target knot. Check its `strand-dir` field.

2. **Verify the knot is registered**: Find the knot in
   `tie-offs/<rig>/state.json` `looms[].knots[]` array. Check its `status` is not `processing`
   (if it is, the knot is already working).

3. **Create or touch a strand file** in the strand directory:
   ```bash
   # Create a new file (triggers Created event)
   echo "content" > <strand-dir>/<filename>.md

   # Touch an existing file (triggers Modified event)
   touch <strand-dir>/<filename>.md

   # Modify an existing file (triggers Modified event)
   echo "updated content" >> <strand-dir>/<filename>.md
   ```

4. **Wait for processing**: The debounce window is 100ms. Processing
   typically completes within seconds to minutes depending on the
   agent's work.

5. **Verify the knot fired**: Check the service log for `KnotProcessing`,
   then `KnotCompleted` (or `KnotFailed`) for the target knot:
   `grep 'knot={knot-name}' tie-offs/<rig>/knot-service.log | tail`.

6. **Check the tie-off output**: Query the latest section for the knot
   in `tie-offs/<rig>/tie-offs.log` and verify it contains the expected
   output — grep, never whole-file read:
   ```bash
   grep -n "knot={knot-name} " tie-offs/<rig>/tie-offs.log | tail
   # then read just the section at the reported line, or follow the
   run's git commit (the commit body carries the full tie-off content)
   ```

### Trigger an Event Consumer Knot (Manual Event Delivery)

A knot whose `strand-dir` is an `event:` URI (e.g.
`event:quality-reviewer:ReviewCompleted`) is triggered by an
`Event`-kind queue entry that carries the delivery payload. There is
**no dispatch directory to drop a file into** (plan 099) — the manual
trigger is a **direct queue write** of the entry itself.

1. **Identify the consumer and the emission you want to simulate**:
   Read the consumer knot's definition file at
   `rig/{loom-id}/{knot-name}.md`. The `strand-dir` event URI gives
   the producer knot and `EventId` (the producer's knot name must
   match what you put in `delivery.producer_knot`).

2. **Write an `Event`-kind entry** straight into the queue
   (`tie-offs/<rig>/events/`) — the shape Knot's own dispatcher
   writes:
   ```bash
   id="$(date +%s%3N)-hand"
   cat > "tie-offs/<rig>/events/${id}.json" << EOF
   {
     "id": "${id}",
     "kind": "Event",
     "loom_id": "{consumer-loom-id}",
     "knot_id": "{consumer-knot}",
     "strand_path": "event:{producer-knot}/{EventId}",
     "queued_at": "$(date -Iseconds)",
     "delivery": {
       "event_id": "{EventId}",
       "producer_knot": "{producer-knot}",
       "producer_loom": "{producer-loom-id}",
       "original_strand": "path/to/producer-strand.md",
       "body": "Manual trigger for testing.",
       "payload": { "description": "hand-written manual event" }
     }
   }
   EOF
   ```
   Field notes: `loom_id` / `knot_id` are the **consumer's** (the
   entry names who runs); `strand_path` is the synthetic
   `event:<producer-knot>/<event-id>` descriptor, not a path;
   `original_strand` and `body` are optional (omit them and the
   fields are absent from the rendered delivery); `payload` is a
   `key: value` map — include `description` plus any fields the
   consumer expects. Filename stem = entry id; FIFO order is filename
   sort, so the `{unix_timestamp_ms}` prefix controls ordering.

3. **Wait for processing** — a running service picks the entry up from
   the queue; or step it explicitly (`knot step --event ${id}` — see
   [Stepping](#stepping-knot-step--one-event-then-stop)). The entry is
   removed from `events/` once the run completes (late removal). No
   cleanup step is needed — the queue is transient.

### Trigger a Producer Knot (Which Emits Events to Consumers)

When a producer knot completes, Knot scans its tie-off for event blocks
and dispatches matching events to consumer knots. To trigger the
full producer→consumer chain:

1. **Trigger the producer** using the normal filesystem trigger above
   (create/touch a file in the producer's `strand-dir`).

2. **Wait for the producer to complete**: Monitor the service log for
   `KnotCompleted` with `loom={producer-loom-id}`.

3. **Check for event dispatch**: Look for `EventsDispatched` entries
   in the service log (filter with `grep 'loom={producer-loom-id}'`).
   These show which events were emitted and which consumer looms
   received their queue entries. If the producer acknowledged every
   subscriber event with `occurred: false`, there will be no
   `EventsDispatched` entry — that is expected, not a failure.

4. **Verify consumer activation**: Read each consumer loom's log for
   `KnotProcessing` entries that follow the dispatch.

5. **Trace the full chain**: The sequence should be:
   ```
   Producer: KnotProcessing → KnotCompleted → EventsDispatched
   Consumer: KnotProcessing → KnotCompleted
   ```

### Verify a Trigger Worked

After triggering any knot:

1. **Check `tie-offs/<rig>/state.json`**: The knot's `status` should be
   `completed` (or `failed` if something went wrong).
   The `last_event_at` field should have a recent timestamp.
   The `last_strand_path` should point to the file you created/touched.

2. **Check the service log**: Tail `tie-offs/<rig>/knot-service.log`
   and look for:
   ```
   [KNOT][EVENT] KnotProcessing loom=… knot=… strand=… [session=… event=… source=… original-strand=…]
   [KNOT][EVENT] KnotCompleted loom=… knot=… strand=… tie-off=… [session=… event=… source=… original-strand=…]
   ```
   The run-trace fields (`session=`, `event=`, `source=`,
   `original-strand=`) ride the run-lifecycle lines when present (Knot
   0.63.0+, plan 099). For a consumer run triggered by a delivered
   emission, `strand=` is the synthetic descriptor
   `event:<producer-knot>/<event-id>` and `tie-off=` points at
   `tie-offs.log`.

3. **Check the tie-off**: The new section at the end of
   `tie-offs/<rig>/tie-offs.log` opens with a structured header line
   (grep — never whole-file read):
   ```
   ## knot={knot-name} event={Modified|…} [event-id={EventId}] strand={strand} timestamp=… [session=…] [source={producer-knot}] [original-strand=…]
   ```
   `None` fields are omitted, never written empty. The
   `session=` value is the pi session ID that produced this section —
   open it with `pi --session-id <id>` to see the exact conversation.
   The same fields ride the matching **git commit trailers**
   (`session:` / `event:` / `source:` / `original_strand:`), which
   stay atomic with the work — query the durable side with
   `git log --grep` / `git interpret-trailers`. Inside the section,
   emitted event blocks are slimmed to one audit line each:
   `- event-audit id=… occurred=… reason=… timestamp=…` — the full
   event bodies live in the commit, not the log.

---

## Stepping: `knot step` — One Event, Then Stop

`knot step` processes **exactly one** queued event and then exits. It
is the manual trigger/observation tool: use it to watch one cycle
unfold, inspect rig state between events, or debug a misbehaving knot
(wrong prompt, runaway event loop, event-enforcement follow-ups)
without letting a long-running service drain the queue back-to-back.

**Use it when the service is NOT running.** Two processes share the
disk queue, and `knot step` is not safe to run concurrently with the
service — the same event can be read twice (double execution). That is
tolerable under knot idempotency but wasteful. To step a queue, stop
the service first (the queue survives on disk — that is the point of
the persistent queue).

```
knot step [--rig <rig-name>] [--event <event-filename>]
```

| Option | Meaning |
|--------|---------|
| `--rig <rig-name>` | Target a specific rig (same `./<name>` resolution as the service's positional). Without it, auto-discovery applies — but **zero matches is an error** (no implicit `rig/` creation) and multiple matches is an error. Stricter than the service on purpose. |
| `--event <spec>` | Target a specific queued event: exact event id (`.json` optional), unique id prefix, or strand filename. No match → the queued events are listed on **stderr** and the exit code is 1. An ambiguous match (multiple ids share the prefix) also lists the queue and exits 1. |
| *(neither)* | The FIFO head is processed. |

**Empty queue** → prints `queue empty`, exit 0, no agent run. Before
declaring the queue empty, the step waits up to 5× the debounce window
(so a just-touched strand can clear its debounce window first).

**Exit codes:** `0` = an event was processed, or the queue was empty;
`1` = error (unknown/ambiguous event, no rigs, multiple rigs,
processing failure).

### What a Step Does

A step runs the **full service startup** — legacy-layout migration,
config seeding, rig git init, loom discovery, watcher registration,
debounce engine, config pipeline, and the 5-second state writer — then
executes exactly one event and shuts down with the same graceful
cascade as the service (in-cycle debounce buffer flushed to disk,
pipelines drained, `LoomStopped` recorded on the service log):

- Events that occur **during** the step (a dispatched agent event,
  the agent modifying its own strand) are captured into
  `tie-offs/<rig>/events/` but **not executed** — the next step (or a
  service run) processes them.
- **Run activity is in-memory per process** (Knot 0.41.0+ — no log
  files are cleared, because there are none to clear): when you run the
  step in the foreground, its `[KNOT][EVENT]` lines on stderr show the
  whole run; when the service is started with `knot-start`, each step's
  lines append to `tie-offs/<rig>/knot-service.log`. Each step is a
  self-contained run, so the event lines carry a fresh
  `KnotRegistered`/`LoomStarted` … `LoomStopped` bracket per step.
- `tie-offs/<rig>/state.json` is written during the step, so it
  reflects the **post-step** queue (processed event removed, the rest
  listed).
- Every step takes at least a few seconds (the pipeline drain has a
  5-second timeout) — expect roughly one service startup per event.
- On an early error (e.g. unknown `--event`) the step exits before the
  shutdown cascade, so the `LoomStopped` bracket is missing from that
  run's event lines — expected on error exits.

### Getting an Event Queued for a Step

The file watcher fires only on changes *after* the watch starts, so a
strand file created before `knot step` begins is **not** picked up
automatically. Two ways to have work queued:

1. **The natural flow** — let the service queue it: run the service,
   create/touch a strand (or let a producer's emissions queue their
   `Event`-kind entries), stop the service before or after the event is
   queued. Whatever is still in `tie-offs/<rig>/events/` survives the
   stop and is what the steps will process.
2. **Direct queue write** (deterministic, watcher-free) — write the
   pending event's JSON file straight into `tie-offs/<rig>/events/`.
   The disk *is* the queue: the step's startup scan
   (`load_persisted`) picks it up. The filename is the event id
   (`{id}.json`); FIFO order is filename sort, so the
   `{unix_timestamp_ms}` prefix controls ordering.

   A filesystem entry:
   ```bash
   id="$(date +%s%3N)-0001"
   cat > "tie-offs/rig/events/${id}.json" << EOF
   {
     "id": "${id}",
     "kind": "Created",
     "loom_id": "planning-loom",
     "knot_id": "plan-creator",
     "strand_path": "/abs/path/to/strand-dir/my-feature.md",
     "queued_at": "$(date -Iseconds)"
   }
   EOF
   ```
   `kind` is `Created`, `Modified`, `Deleted` (file pointers —
   `strand_path` is the strand file's absolute path; for
   `Created`/`Modified` the strand file must exist, a missing file is
   `StrandSkipped`; for `Deleted` the file is expected to be gone), or
   **`Event`** (a payload-bearing delivery — the shape is in
   [Trigger an Event Consumer Knot](#trigger-an-event-consumer-knot-manual-event-delivery);
   it is the sole manual event dispatch, plan 099 D8). `knot step
   --event <id>` addresses entries of either kind by id (exact /
   prefix / strand-filename resolution unchanged).

### Agent Workflow — Step Through the Queue

When asked to step, process one event, or observe a single cycle:

1. **Ensure the service is not running**: `tie-offs/<rig>/state.json`
   `updated_at` is not fresh. If it is running, stop it with `kill -INT`
   (see `knot-start` — Knot only handles SIGINT; the queue survives the
   stop).
2. **Check what is queued**: `ls tie-offs/<rig>/events/` and/or the
   `strand_queue` array in `state.json`.
3. **Queue work if needed** (see above — direct queue write is the
   deterministic path).
4. **Run the step**:

   ```bash
   knot step                    # FIFO head
   knot step --rig dev-rig      # named rig
   knot step --event 1750000000000   # unique id prefix
   knot step --event my-feature.md   # strand filename
   ```

   To keep one cross-run record, append the step's output to the service
   log instead (the per-run logs are cleared on the next start) —
   `pipefail` preserves Knot's exit code through `tee`:

   ```bash
   set -o pipefail
   knot step 2>&1 | tee -a tie-offs/rig/knot-service.log
   ```
5. **Observe the cycle**:
   - stdout: `[step] processing event <id> (loom=…, knot=…): <path>`,
     then `[step] event <id> processed` (or `failed: …` on stderr).
   - service log (stderr): `KnotProcessing` → `KnotCompleted` (or
     `KnotFailed`), `StrandProcessed`, and `EventsDispatched` if the
     knot emitted events.
   - tie-off: the new section at the end of `tie-offs/<rig>/tie-offs.log`
     (grep by `knot=…` — never whole-file read).
   - queue: the processed event's file is gone from
     `tie-offs/<rig>/events/`; anything queued *during* the step (new
     filesystem entries, or `Event`-kind deliveries the stepped knot
     emitted) now has a new file there (captured, not executed).
   - `state.json`: reflects the post-step queue.
6. **Repeat** to step through the rest of the queue. When it prints
   `queue empty` (exit 0), the queue is drained — start the service
   again for continuous processing, or leave the rig idle.

---

## Troubleshooting

### Knot Did Not Fire

| Symptom | Check |
|---------|-------|
| No `KnotProcessing` in the service log | Verify Knot is running (start it with `knot-start` if `tie-offs/<rig>/state.json`'s mtime is old and no run is in flight). Check `tie-offs/<rig>/knot-service.log` for `[KNOT][EVENT]` errors around the trigger time. |
| `KnotProcessing` but no `KnotCompleted` | Agent may have timed out. Check the service log for `TimeoutExceeded`. Increase profile `timeout`. |
| `KnotFailed` in the service log | Read the `error=` field of the line. Common causes: missing profile, parse errors, agent crash. |
| File created but no event at all | The strand directory may not be watched. Check `tie-offs/<rig>/state.json` — is the loom and knot registered? Knot may need a restart to pick up new watches. |
| `StrandSkipped` in the service log | See the table below. The strand event was queued but the file was not available when processing reached it (filesystem entries only — `Event`-kind entries carry their payload and never skip on a missing file). |
| Manual `Event`-kind entry written but consumer did not fire | Verify the JSON is valid and the entry loaded (`[startup] loaded N persisted event(s)` or the step's `processing event <id>` line), `kind` is `Event` with a `delivery` object, and `loom_id` / `knot_id` name the **consumer**. An unknown/ambiguous `knot step --event` lists the queued ids on stderr. |

### StrandSkipped — File Vanished Before Processing

`StrandSkipped` appears in the service log when a strand event was queued
but the file could not be processed. Two variants exist:

| Reason | Cause | Action |
|--------|-------|--------|
| `"filtered temp file"` | A known temp file pattern (e.g. `sedXXXXXXX` from `sed -i`) triggered the watcher. The file was never a real input — it's normal filesystem noise. | No action needed. Count these to gauge noise levels. |
| `"missing file (unknown pattern)"` | A real file triggered a `Created` or `Modified` event but was deleted before `ProcessStrand` reached it. This is a race condition: the file watcher fires instantly, but the file may be short-lived (a script creates, reads, and deletes it within milliseconds). | If one-off: no action — the event is removed from the queue at the point of failure. If recurring for the same path: investigate what is creating and deleting files in the strand directory. |

The event file in `tie-offs/<rig>/events/*.json` is removed at the point
of failure — the `StrandSkipped` does not recur from the same queued
event. It only repeats if the file watcher generates a *new* event for
the same path.

For `Deleted` strand events, the file check is skipped entirely
(the file being gone is expected), so `StrandSkipped` never fires
for `Deleted` events.

### Triggering Too Frequently

If the same strand keeps re-triggering the same knot:

- **Debounce**: Rapid edits are coalesced. Only one `Modified` event
  fires after the debounce window closes (100ms after last change).
- **Idempotency**: Knots should be idempotent (see `knot-design`
  skill). Re-triggering on the same strand should produce
  "no changes needed" after the first run.
- **Strand pinning**: If you need to prevent re-processing, the
  strand file should not change after the initial trigger.

### Event Not Reaching Consumer

| Symptom | Check |
|---------|-------|
| Producer emitted event but consumer idle | Check `EventsDispatched` in the producer's service-log lines — does it list the consumer's loom? If not, the consumer's `strand-dir` event URI may not match the producer's knot name. |
| `EventsDispatched` logged but no consumer run | The consumer's queue entry should exist until the run completes — if `events/` is empty and no `KnotProcessing` for the consumer, check the service log around the dispatch for a `KnotFailed` / `StrandSkipped` (a failed delivery removes its entry at the point of failure). |
| `occurred: false` in tie-off but expected event | The producer explicitly acknowledged the event but determined it did not occur. Read the `description` field in the event block (commit body / session) — it states **why** the event wasn't triggered. If the reasoning is wrong, re-trigger the producer. |
| `KnotEventsMissing` in the service log | The producer completed with **zero** event blocks. The entry lists `expected_events`. Knot already attempted one follow-up re-entry (a second `KnotEventsMissing` means the follow-up also produced nothing). Re-trigger the knot if the events are still missing. |
| Multiple consumers, only some fired | Each subscriber gets its own queue entry — fan-out is per-entry, so one failing run does not hold back the others. Check each consumer's `strand-dir` event URI against the producer's knot ID, and each consumer's `KnotProcessing` line. |

---

## Manual Event Queue Entry Format

The manual event trigger is the **direct queue write** (Knot 0.63.0+,
plan 099): a hand-written `Event`-kind entry in
`tie-offs/<rig>/events/`. There are no dispatched event *files* any
more — the entry **is** the message. The full field-by-field shape and
field notes are in
[Trigger an Event Consumer Knot](#trigger-an-event-consumer-knot-manual-event-delivery);
the canonical example:

```json
{
  "id": "9000000000001-hand",
  "kind": "Event",
  "loom_id": "review-loom",
  "knot_id": "review",
  "strand_path": "event:plan-creator/PlanCreated",
  "queued_at": "2026-01-01T00:00:00",
  "delivery": {
    "event_id": "PlanCreated",
    "producer_knot": "plan-creator",
    "producer_loom": "plan-loom",
    "original_strand": "project/plans/plan-001.md",
    "body": "A new plan was created covering three phases.",
    "payload": {
      "description": "Hand-written plan created",
      "plan": "PLAN-001"
    }
  }
}
```

The consumer's prompt receives the rendered delivery —
`## Event: PlanCreated from plan-creator (loom plan-loom)`, the
`original_strand:` line, the payload fields as `- **key**: value`, and
the body — everything the consumer used to read from a dispatched
file, delivered from the queue entry instead. The entry is removed
from `events/` once the run completes; `knot step --event
9000000000001-hand` steps it by id (service stopped).

---

## Producer Event Emission

When a knot has consumers listening for its events, Knot injects
subscriber event instructions into the agent's **system prompt**
(via `--append-system-prompt`, Knot 0.66.0+ — so they survive
compaction). The
injected block lists each declared event and requires the producer to
**acknowledge every subscriber event** in its tie-off — whether or not
the event occurred:

````
# Subscriber Events

You have a number of subscribers that have requested to be notified if
certain events occur during this session. You must acknowledge each
event in your tie-off (final response). Subscribers can't begin their
work until your events are delivered to them via your tie-off.

The following event/s have been declared by subscribers:

- `PlanCreated` — When a plan is created.
- `RefactorNeeded` — When the review finds architectural issues.

## Event Format

Emit one ```markdown block **per subscriber event** listed above.
Each block must have `---` frontmatter delimiters:

```markdown
---
event: PlanCreated
occurred: true
description: Short summary of what happened
timestamp: 2026-08-06T14:30:00
<optional fields if specified in the event description above>
---

Freeform narrative context about the event.
```

## Rules

- Emit exactly one event block per subscriber event listed above.
- The `event` and `occurred` fields are required in every block.
- The `description` field must explain why the event was or wasn't
  triggered, plus any additional requested information.
- When `occurred: true`, include the `timestamp` field and any
  additional fields specified in the event description above.
- When `occurred: false`, the event is not dispatched but still
  counts as acknowledgement.
- If a pending event satisfies the event that has just occurred, set
  occurred: false to avoid duplicated events.
- A pending event may already be relevant but require additional
  context — do not edit the pending event and instead emit a new
  event of the same type with additional context.
````

(Shown with cleaned-up wording — the runtime block lists the real
subscriber events and their descriptions, and appends a
`## Queued Events` section when events of your emittable types are
still queued, see below.)

### The Acknowledgement Convention

This replaces the older `event: None` convention: **silence is no
longer a valid outcome.** Every declared event must be answered
explicitly with its own block, and **when an event did not occur, the
knot must state why** in the `description` field.

| Field | Required | When | Meaning |
|-------|----------|------|---------|
| `event` | **Yes** | every block | The subscriber event id (e.g. `PlanCreated`) |
| `occurred` | **Yes** | every block | `true` = happened during the session; `false` = did not |
| `description` | **Yes** | every block | **Why** the event was or wasn't triggered, plus any additional requested information |
| `timestamp` | **Yes** | `occurred: true` | When the event occurred |
| payload fields | as specified | `occurred: true` | Any fields named in the event description |

An `occurred: false` block is **not dispatched** to consumers — it
exists solely as acknowledgement of the subscriber requirement. Its
`description` is the audit trail that distinguishes a conscious
decision from a forgotten requirement:

````markdown
---
event: RefactorNeeded
occurred: false
description: Review found no architectural issues — nothing to refactor
---
````

### Queued Events

When the producer has previously dispatched events that are still
queued for consumer processing, the injected block also contains a
**`## Queued Events`** section (Knot 0.61.0+): per emittable event
type, the full count plus the newest capped entries (path + queued
at) — read from the persistent queue, not the event bodies. Two
dedup rules apply (the norm itself lives in the event-content guide
of the tie-off instructions):

- **Covered outcome:** if a queued event already carries the same
  facts (compare frontmatter keys such as plan/phase), emit the event
  with `occurred: false` (description referencing the queued event)
  instead of queueing a second signal.
- **Needs more context:** if a queued event is relevant but requires
  additional context, do **not** edit the queued event file — emit a
  new event of the same type with the additional context.

### Enforcement

If the producer completes with **zero** event blocks despite having
subscribers, Knot does not silently accept the tie-off:

1. Records `KnotEventsMissing` as a service-log event, listing the
   `expected_events`.
2. Attempts one follow-up re-entry into the agent session (when a
   session ID is available), re-injecting the event request.
3. If the follow-up produces event blocks, they are parsed and
   dispatched as normal (only one `KnotEventsMissing` is logged).
4. If the follow-up still produces nothing, a second
   `KnotEventsMissing` is logged.

An `occurred: false` block is a valid event block — its presence
satisfies enforcement.

### Dispatch (Direct Queueing)

Knot parses the producer's tie-off for event blocks and queues only
`occurred: true` events **directly** — one payload-bearing `Event`-
kind entry per subscriber (plan 099). `occurred: false` events are
filtered out before queueing. Multiple events in one tie-off each
produce their own fan-out. The payload is embedded in the entry — no
dispatched file, no watched directory; the durable copies are the
producer's git commit (full event block + trailers) and the slim
`- event-audit …` line in `tie-offs.log`.

The producer does **not** need to declare its events — Knot discovers
them from consumer subscriptions.

### Backwards Compatibility

- The `occurred` field **defaults to `true`** when absent, so event
  blocks written before this convention (without the field) are still
  parsed and dispatched.
- A legacy `event: None` block parses as a literal event with id
  `None` — it satisfies enforcement (a block is present) but matches
  no consumer, so it is never dispatched.

---

## Quick Reference

```bash
# Trigger a normal knot (filesystem strand-dir)
echo "work item" > project/prds/new-feature.md

# Trigger an event consumer (manual event delivery — direct queue write)
id="$(date +%s%3N)-hand"
cat > "tie-offs/rig/events/${id}.json" << EOF
{
  "id": "${id}",
  "kind": "Event",
  "loom_id": "planning-loom",
  "knot_id": "refactor-planner",
  "strand_path": "event:quality-reviewer/ReviewCompleted",
  "queued_at": "$(date -Iseconds)",
  "delivery": {
    "event_id": "ReviewCompleted",
    "producer_knot": "quality-reviewer",
    "producer_loom": "review-loom",
    "body": "Manual review trigger.",
    "payload": { "description": "hand-written manual event" }
  }
}
EOF

# Verify trigger worked — check the service log
grep 'loom=planning-loom' tie-offs/rig/knot-service.log | tail -5

# Verify trigger worked — check state
cat tie-offs/rig/state.json | python3 -c "
import sys, json
state = json.load(sys.stdin)
for loom in state['looms']:
  for knot in loom['knots']:
    print(f\"{loom['id']}/{knot['id']}: {knot['status']} (last: {knot.get('last_event_at', 'never')})\")"

# Check tie-off output (the unified log — grep, never whole-file read)
grep -n "knot=refactor-planner " tie-offs/rig/tie-offs.log | tail -3
grep EventsDispatched tie-offs/rig/knot-service.log

# Step one queued event (service must NOT be running)
knot step

# Target a specific rig / queued event
knot step --rig dev-rig
knot step --event 1750000000000        # exact id, .json optional, or unique prefix
knot step --event my-feature.md        # or strand filename

# Empty queue → "queue empty", exit 0; unknown event → queue listed
# on stderr, exit 1
```

---

## Cross-Reference

**Before using this skill:** Read the `knot-abstractions` skill for the
layered architecture overview. Understanding how strands, events, the
queue, and ties work together is essential for triggering knots
correctly.

Related skills:

1. **knot-abstractions skill** — foundational architecture overview
2. **knot-create skill** — create looms, knots, and profiles (must exist before triggering)
3. **knot-inspect skill** — inspect rig state and verify trigger results
4. **knot-design skill** — design knots with correct event contracts
5. **knot-manage skill** — review the work produced by triggered knots

This skill covers **triggering** — getting knots to run, including
single-event stepping with `knot step`. The other skills handle setup,
monitoring, and design.

Related docs: `docs/concepts.md` (Event Queue — at-least-once
semantics and `knot step`), `docs/troubleshooting.md`.
