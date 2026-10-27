---
name: knot-manage
description: "Review the rig's work using git history and the unified tie-off log. Examine what the rig has produced, assess the quality of agent output, trace the interaction chain between producer and consumer knots to verify communication was effective, and identify gaps between what was triggered and what was delivered."
license: MIT
metadata:
  author: Knot Team
  version: "2.1.0"
  compatibility: "Knot 0.64.0+"
---

# Knot Manage Skill

Review the work produced by a Knot rig. This skill uses git history and
the unified tie-off log to assess what the rig has done, how effectively
knots communicate with each other, and whether the produced output
matches intent.

Unlike `knot-analyst` which monitors **live rig operations** (timeouts,
processing rates, blockers), this skill performs a **retrospective
review** of completed work: what was written, what was communicated,
and whether the interaction chain achieved its goal.

**Tie-off output:** `tie-offs/<rig>/tie-offs.log` — one accumulating
log, the service is the sole writer; every knot's every run appends one
section. It is a **log, not a state file**: query it (grep by knot/loom/
`event-id=`), never whole-file read it; git is the rotation.
**Service log:** `tie-offs/<rig>/knot-service.log` (single-line
`[KNOT][EVENT]` / `[KNOT][STATE]` records, appended across runs by
`knot-start`; filter per loom with `grep 'loom=<id>'`) — since Knot
0.41.0 there are no per-run log files
**Git commits:** Knot-generated commits follow the pattern
`knot: <knot-id> — processed <strand-name> (<event-type>)` and carry a
trailer block (`session:`, `event:`, `source:`, `original_strand:` —
Knot 0.63.0+, plan 099) that is the durable trace, atomic with the work

Since Knot 0.31.0 there are **two independent git repositories**:

- **Project git** — the audit trail. Contains the runtime tree
  (`tie-offs/<rig>/`: `tie-offs.log`, the appended service log, event
  queue, `state.json`) plus everything the knots wrote to the project
domain. Knot's per-knot-run commits live here.
- **Rig git** (`rig/.git`) — the rig source (looms, knots, profiles,
  config). Committed **manually by the user**; Knot never commits it and
  project commits never touch it (parent `.gitignore` exclusion).

When reviewing rig *definitions* (knot instructions, profile changes),
look at the rig git: `git -C rig log --oneline -20`.

---

## Core Philosophy

### Git Commits Are the Fidelity Source

When `git-versioned` is `true` (the default), every successful knot run
creates a git commit. The commit subject identifies the knot and strand;
the body contains the tie-off content (incl. the full event blocks — the
commit is the fidelity source); the last-paragraph trailer block
(`session:` / `event:` / `source:` / `original_strand:`) links the run
to its pi session and its trigger. This means `git log` provides a
complete, searchable history of all rig activity — independent of the
rig itself. Query the durable side with `git log --grep`, and read a
single run's full output with `git show <hash>`.

### `tie-offs.log` Is Append-Only History

One log accumulates **every knot's** sections (`tie-offs/<rig>/tie-offs.log`;
Knot 0.63.0+, plan 099 — the per-knot
`tie-off-{knot-name}.md` files retired). Each section opens with a
structured header line (`## knot=… event=… [event-id=…] strand=…
timestamp=… [session=…] [source=…] [original-strand=…]`); the emitted
event blocks are slimmed to one `- event-audit id=… occurred=…
reason=… timestamp=…` line each (no payload bodies — those live in the
commit). Read norm: **query, never whole-file read** — grep for the
knot's header line, then read just that section; for the full narrative
of a run follow `session=` to `pi --session-id` or the commit body.

### Interaction Review Is About Signal Quality

When knots communicate via events, the quality of that communication
determines rig effectiveness. This skill reviews:

1. **Was the event emitted?** (producer's commit body contains the event
   block; the `tie-offs.log` section carries the slim audit line)
2. **Was the event delivered?** (a consumer run was triggered from a
   queue entry — `KnotProcessing … strand=event:<producer>/<event-id>`
   for the consumer in the service log)
3. **Did the consumer react correctly?** (consumer section in
   `tie-offs.log` / commit shows meaningful output, not "no changes
   needed" or errors)
4. **Is the event payload sufficient?** (consumer had enough facts
   to do its job without instructions)

---

## Prerequisites

1. The project must be in a git repository with Knot-generated commits.
   Verify with: `git log --oneline --grep="knot: " | head -10`
   If no Knot commits exist, either `git-versioned` is `false` on
   knots or the rig has not produced successful output yet.
2. `tie-offs/<rig>/tie-offs.log` must exist (the service creates it on
   the first tie-off). Check `tie-offs/<rig>/` for content.

---

## Agent Workflow

### Review All Rig Work (Git History)

When asked to review what the rig has done, or check recent rig output:

1. **Show recent Knot commits**:
   ```bash
   git log --oneline --grep="knot: " -20
   ```

   Each line follows the pattern:
   ```
   <hash> knot: <knot-id> — processed <strand-name> (<event-type>)
   ```

2. **Analyse the commit pattern**:

   | Signal | Interpretation |
   |--------|---------------|
   | Many commits from same knot on same strand | Knot is re-triggering frequently. Check tie-off for "no changes needed" (idempotent convergence) or actual repeated work (possible loop). |
   | Commits span multiple knots | Healthy — different knots are producing output. |
   | Commits touch expected output directories | Knots are doing their job (writing plans, docs, code). |
   | Commits only touch `tie-offs/<rig>/` | Knots are producing tie-offs but not writing to their target domains. Check knot instructions — they may not be told where to write output. |
   | `rig/` appears in a project commit | Should not happen since 0.31.0 (parent `.gitignore` + rig unstaging). The project predates the migration — run the one-time `git rm -r --cached rig/` untrack step. |
   | No Knot commits in last 24h | Rig is idle or all work is complete. |

3. **Show files modified by Knot commits**:
   ```bash
   git log --name-only --grep="knot: " -10
   ```

   This reveals which directories the rig is actually writing to.

4. **Produce a summary**:
   ```
   ## Rig Work Review (last 24h)

   Knot commits: 15
   Active knots: goals-review (8), prd-planner (5), adr-review (2)
   Output directories: project/plans/ (7), project/adrs/ (3), project/prds/ (5)
   Assessment: Active — planning knot is most productive.
   ```

### Review a Specific Knot's Work

When asked to review what a specific knot has produced:

1. **Query the knot's sections** in `tie-offs/<rig>/tie-offs.log`
   (grep — never whole-file read):
   ```bash
   grep -n "^## knot={knot-name} " tie-offs/<rig>/tie-offs.log | tail -20
   ```

2. **Read the sections**: each starts at its structured header line
   (Knot 0.63.0+, plan 099):
   ```
   ## knot={knot-name} event={Created|Modified|…} [event-id={EventId}] strand={strand-path} timestamp=… [session=…] [source={producer}] [original-strand=…]
   ```
   followed by the agent output (with event blocks slimmed to
   `- event-audit …` lines). `None` fields are omitted, never written
   empty. `event=` is the run's kind label — `Event` for a delivered
   emission (the emission's id rides in `event-id=`), else the
   filesystem trigger (`Created` / `Modified` / `Deleted`).
   For the full narrative of a run, follow `session=` to
   `pi --session-id <id>`, or read the run's commit body (`git
   show <hash>`) — the commit is the fidelity source.

3. **Assess each section**:

   | Criterion | What to Look For |
   |-----------|-----------------|
   | **Goal alignment** | Does the output address the strand's intent? (Read the strand if available to compare.) |
   | **Idempotency** | Repeated runs on the same strand should show "no changes needed" after the first meaningful run. |
   | **Output quality** | Is the output substantive (actual changes, analysis, decisions) or superficial (summaries, restatements)? |
   | **Constraints respected** | Did the knot stay within its responsibility boundary? (Check it didn't modify files outside its domain.) |
   | **Event emission** | If the knot is a producer, does the tie-off contain event blocks? Are they factual (not instructional)? |

4. **Cross-reference with git**:
   ```bash
   git log --oneline --grep="knot: {knot-name} " -10
   ```
   Verify that `tie-offs.log` sections correspond to actual commits
   (a section may have no commit when `git-versioned` is off for that
   knot, or the run failed after writing nothing — failures leave no
   section either; check the service log for `KnotFailed`).

5. **Produce a review**:
   ```
   ## Knot Review: {knot-name}

   Total processing events: 12
   Meaningful outputs: 8 (6 wrote changes, 2 reported "no changes")
   Stale re-triggers: 2 (same strand, no new work)
   Events emitted: 3 (PlanCreated × 2, ScopeChanged × 1)
   Output quality: Good — changes are targeted and idempotent.
   Concern: 2 re-triggers on the same PRD without new work —
   strand has not been updated since last processing.
   ```

### Tie-Off Event Blocks vs. Queue Deliveries

When reviewing interaction chains, distinguish three different
artifacts — confusing them is a common source of false "dead
subscription" diagnoses (Knot 0.63.0+, plan 099):

**Event blocks in the producer's commit body:**
These are ```markdown fenced code blocks the producer agent writes as
part of its output (the final response *is* the tie-off). They *declare*
that an event occurred. Knot parses these blocks **after** the producer
completes and queues the matching events directly. If the producer
writes no block for an expected event, nothing is queued — the
producer agent simply didn't emit it (either no event was warranted, or
the agent was not instructed to emit that event type). The full blocks
persist in the **commit body** (the fidelity source) and, in the
`tie-offs.log` section, are slimmed to one
`- event-audit id=… occurred=… reason=… timestamp=…` line each.

**Queue entries** (`tie-offs/<rig>/events/*.json`, `kind: "Event"`):
These are the deliveries — one per subscriber, payload embedded in a
`delivery` object, removed from the queue once the consumer's run
completes (late removal). They are transient: an empty `events/`
**after** a chain ran is the normal, healthy end state. The durable
evidence of a delivery is the consumer's own run — its
`KnotProcessing … strand=event:<producer>/<event-id>` service-log line
and its commit/`tie-offs.log` section.

**Slim audit lines** (`- event-audit …` in `tie-offs.log`):
The producer-side record that an emission was parsed — id, occurred,
reason (the block's `description`), timestamp. No payload. Used for
fast "did the producer acknowledge every subscriber event" checks:
grep the producer's sections for the expected `id=` values.

The producer **never queues anything itself**. The flow is:

1. Producer agent writes event blocks in its tie-off (final response)
2. Knot commits the run (commit body carries the full blocks +
   `event:` / `source:` / `session:` / `original_strand:` trailers)
   and appends the section to `tie-offs.log`
3. Knot parses the blocks and writes one `Event`-kind queue entry per
   subscriber (`EventsDispatched` service-log line)
4. Consumer runs from its queue entry (payload rendered into the prompt)

If step 1 never happened, steps 2–4 never occur for that event.

---

### Review an Interaction Chain (Producer → Consumer)

When asked to review how well two knots communicated, or whether an
event chain achieved its goal:

1. **Identify the chain**: Determine which knot is the producer and
   which is the consumer. Read their definition files:
   ```
   rig/{producer-loom-id}/{producer-knot-name}.md
   rig/{consumer-loom-id}/{consumer-knot-name}.md
   ```

   The consumer's `strand-dir` should be an `event:` URI referencing
   the producer.

2. **Trace the producer's emission**:
   Query the producer's sections in `tie-offs/<rig>/tie-offs.log`
   (`grep -n "^## knot={producer-knot-name} " tie-offs/<rig>/tie-offs.log`)
   and the producer's commits. For each run that carries an event
   (`- event-audit id={EventId} occurred=true …` line, or the full
   ```markdown block with `event: {EventId}` in the commit body):
   - Record the event ID, payload fields (from the commit body — the
     log line carries only `reason`), and body text.
   - Note which strand triggered this processing event (`strand=` in
     the section header).

3. **Trace the consumer's reaction**:
   Find the consumer's run that the delivery triggered — its service-
   log line is `KnotProcessing … knot={consumer-knot} …
   strand=event:{producer-knot}/{EventId}` and its `tie-offs.log`
   section header carries `event-id={EventId}`, `source={producer-knot}`,
   and `original-strand=` (the producer's trigger strand). Cross-check
   the matching commit trailers (`event: {EventId}`, `source:`,
   `session:`, `original_strand:` — the session is **identical** on the
   section header and the trailer). Then assess the consumer's output
   (section body / commit body / `pi --session-id`): did it react
   meaningfully to the event?

4. **Check delivery evidence** (no dispatch directories exist any
   more):
   - service log: `EventsDispatched` for the producer, then
     `KnotProcessing … strand=event:{producer-knot}/{EventId}` for the
     consumer
   - `tie-offs.log`: the consumer's section with `event-id={EventId}`
   - `events/`: transient — entries are removed at the consumer's
     completion, so an empty `events/` says nothing about whether a
     delivery happened; the log/commit evidence above is authoritative

5. **Assess interaction quality**:

   | Dimension | Good | Poor |
   |-----------|------|------|
   | **Event emission** | Producer emits events when warranted, with factual payload | Producer emits `event: None` when something significant happened, or emits events for trivial changes |
   | **Event payload** | Payload contains facts the consumer needs (identifiers, values, descriptions) | Payload is empty or contains only a timestamp — consumer must guess context |
   | **Event instructions** | Payload provides context, not commands | Payload contains "you should do X" — the event has absorbed the consumer's responsibility |
   | **Consumer reaction** | Consumer reads event, investigates its domain, makes targeted changes | Consumer reports "no changes needed" for every event, or makes broad unrelated changes |
   | **Timeliness** | Consumer processes event shortly after dispatch | Large gaps between producer emission and consumer processing |
   | **Completeness** | All producer events have corresponding consumer processing | Some events queued but the consumer's `tie-offs.log` sections have no matching `event-id=` entry |

6. **Produce an interaction review**:

   ```
   ## Interaction Review: {producer} → {consumer}

   Event type: {EventId}
   Events emitted: 5
   Events consumed: 5 (all dispatched events were processed)
   Payload quality: Good — events carry plan ID, description, and
     relevant context without instructions.
   Consumer reaction: Effective — consumer made targeted changes
     to project plans in 4 of 5 cases. 1 event resulted in
     "no changes needed" (plan was already aligned).
   Timeliness: Consumer processed all events within the same
     processing cycle.
   Assessment: Communication is effective. The event contract is
   well-defined and both sides respect it.
   ```

### Review All Interaction Chains

When asked to review the full rig's communication patterns:

1. **List all event subscriptions**:
   ```bash
   grep -r "strand-dir:.*event:" rig/*-loom/
   ```

   This shows all consumer→producer wiring.

2. **For each chain**, perform the interaction review above.

3. **Identify communication gaps**:
   | Gap | Detection |
   |-----|-----------|
   | **Unanswered events** | Producer emitted events (`- event-audit … occurred=true` lines / commit blocks) but the consumer's sections show no matching `event-id=` entries. Check the service log for `KnotProcessing` on those event strands (`grep 'strand=event={producer}/' tie-offs/<rig>/knot-service.log`). |
   | **Dead subscriptions** | Consumer subscribes to an event but the producer never emits it. The producer's sections carry no `- event-audit id={EventId} occurred=true` line, and there are no `EventsDispatched` lines naming the consumer's loom. |
   | **Oscillating chain** | Producer and consumer alternate changes on the same file, each re-triggering the other. Tie-offs show repeated "changes applied" without convergence. |
   | **Over-broadcast** | Producer emits the same event for every minor change. Consumer processes many events that result in "no changes needed." |

4. **Produce a comprehensive review**:

   ```
   ## Full Interaction Review

   Active chains: 3
   1. quality-reviewer → refactor-planner (ReviewCompleted): 8 events, all consumed. Effective.
   2. prd-planner → documentation-loom (PlanCreated): 5 events, 4 consumed, 1 missed (consumer timed out).
   3. adr-planner → planning-loom (ADRUpdated): 0 events emitted. Dead — producer never emits this event type.

   Issues:
   - Chain 2: 1 missed event due to consumer timeout. Increase consumer profile timeout.
   - Chain 3: Dead subscription. Either the producer knot should be instructed to emit
     ADRUpdated events, or the consumer subscription should be removed.
   ```

### Review Git Commit Quality

When asked to review the quality of Knot-generated git commits:

1. **Show commit messages**:
   ```bash
   git log --grep="knot: " -10 --format="%h %s%n%b%n---"
   ```

   Each commit has:
   - **Subject**: `knot: <knot-id> — processed <strand-name> (<event-type>)`
   - **Body**: The tie-off content (truncated to 1000 lines)
   - **Trailers** (Knot 0.63.0+, plan 099): the last-paragraph block —
     `session:`, `event:`, `source:`, `original_strand:` (each omitted
     when absent) — git's own trailer convention, parseable with
     `git interpret-trailers`, and the durable trace atomic with the
     work

2. **Assess commit quality**:

   | Criterion | Good | Poor |
   |-----------|------|------|
   | **Subject clarity** | Identifies which knot, which strand, and the event type | Generic or missing knot/strand info |
   | **Body substance** | Contains the agent's actual output — decisions, analysis, changes made | Empty body, error messages only, or "no changes needed" for every commit |
   | **Atomicity** | Each commit represents one strand processing event | Multiple strands bundled into one commit |

3. **Check commit diff**:
   ```bash
   git show --stat <hash>
   ```

   Verify the commit touches expected files (the knot's target domain)
   and does not inadvertently modify unrelated files.

---

### The Review Surface Is Local Even for a Shim Rig (Knot 0.64.0)

A rig may be a **shim** — its workflow (looms, knots, profiles) served
from an external rig directory (`external-rig:` in
`rig/.workspace-agent-config.yaml`). The review surface is unchanged:
commits, `tie-offs.log`, and `state.json` all live in the **consuming
project**. The only difference is provenance — `state.json`'s
`external_rig` field names the external directory the workflow came
from, and the workflow definitions themselves live (and are reviewed)
in that external rig's own git history, not the project's.

## Tie-Off Section Format (`tie-offs.log`)

`tie-offs/<rig>/tie-offs.log` is the one accumulating tie-off log —
append-only, the service is the sole writer. Every knot run appends one
section (Knot 0.63.0+, plan 099; the per-knot
`tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md` files retired — if an
older rig has them, startup logs `[KNOT][CONFIG]` deprecation
diagnostics and the service never reads or writes them again):

```markdown
## knot=prd-planner event=Created strand=project/prds/feature.md timestamp=2026-07-24T12:00:00Z session=1f2e3d4c-... source=quality-reviewer original-strand=project/plans/plan-001.md
Agent output for this processing event...

- event-audit id=PlanCreated occurred=true reason=New implementation plan timestamp=2026-07-24T12:05:00
## knot=prd-planner event=Modified strand=project/prds/other.md timestamp=2026-07-24T13:00:00Z
Agent output for the second event...
```

The example header above mixes the fields; in practice **`None` fields
are omitted, never written empty** — a plain filesystem trigger carries
only `knot=`, `event=`, `strand=`, `timestamp=` (plus `session=` when
captured); a delivered emission adds `event-id=`, `source=`, and
`original-strand=`, and its `strand=` is the synthetic descriptor
`event:<producer>/<event-id>`.

**Section header fields:**

| Field | Description |
|-------|-------------|
| `knot=` | The knot's identifier |
| `event=` | The run's kind label: `Created`, `Modified`, `Deleted` (a filesystem trigger), or **`Event`** for a delivered emission — the emission's id then rides in `event-id=` |
| `event-id=` | Present when the run was triggered by a delivered emission — the emitted event id (e.g. `PlanCreated`) |
| `strand=` | The trigger: the strand file path, or the synthetic `event:<producer-knot>/<event-id>` descriptor for delivered emissions |
| `timestamp=` | ISO 8601 timestamp of the run |
| `session=` | Present when the pi session ID was captured — the ID of the pi session that produced this section; resume/inspect it with `pi --session-id <id>`. The matching commit trailer carries the identical value |
| `source=` | Present when event-triggered — the producing knot's name |
| `original-strand=` | Present when event-triggered — the original strand that caused the upstream event |

**Slim event-audit lines within a section:**

When a knot is a producer, its emitted event blocks are *not*
reproduced in the log — each becomes one slim audit line:
`- event-audit id=… occurred=… reason=… timestamp=…` (no payload
bodies; the commit body stays the fidelity source). The full blocks
remain in the commit and the session narrative.

---

## Error Handling

| Scenario | Action |
|----------|--------|
| No git repository | Project may not be in git. Skip git analysis, review tie-offs only. Note in report. (Independent: a missing `rig/.git` only means the rig source is unversioned — review still works.) |
| `rig/` tracked by project git | Pre-0.31.0 layout. Run the one-time `git rm -r --cached rig/` + commit untrack step (see knot-init), then verify `git check-ignore -v rig/`. |
| No Knot commits | Either `git-versioned: false` on all knots, or the rig has not produced successful output. Check `tie-offs.log` sections directly. |
| `tie-offs.log` missing | No successful knot run has completed yet (the service creates the log on the first tie-off append). Not an error — the rig may be idle. |
| No sections for a knot | Knot has not yet produced output. Not an error — knot may not have been triggered. |
| Git user not configured | Commits may fail silently. Check `git config user.email` and `git config user.name`. |

## Service-Log Entries to Ignore During Review

When reviewing the service log (`tie-offs/<rig>/knot-service.log`), these
entries are expected and do not indicate problems:

- **`StrandSkipped` with reason `"filtered temp file"`** — A temp file from
  `sed -i` (macOS/Linux), or similar in-place editor triggered a filesystem
  event but was filtered by name before processing. These are recorded for
  completeness only. The strand was never a real input file. Count them to
  gauge filesystem noise levels, but do not flag as issues.

- **`StrandSkipped` with reason `"missing file (unknown pattern)"`** — A file
  triggered a filesystem event (created or modified) but was deleted before
  Knot got to process it. This is a normal race condition: the file watcher
  fires instantly when a file appears, but the file may be short-lived
  (e.g. a script creates it, reads it, and deletes it within milliseconds).
  The event is persisted in the queue (`tie-offs/<rig>/events/*.json`), and when
  `ProcessStrand` reaches it, the file is already gone. The event file is
  removed from the queue at the point of failure, so this does not recur from
  the same event. If you see many of these for the same path, investigate what is
  creating and deleting files in the strand directory.

- **`StrandIgnored` with reason `"binary file"`** — A binary file appeared in
  a strand directory. Knot skips non-text files to avoid passing binary data
  to the agent. Not an error unless binary files should not be in the strand
  directory at all (e.g. a build artifact leaked in).

## File Watcher and Pre-Existing Files

The `notify` file watcher only fires events for changes that occur *after*
`watch()` is called. It does not scan the watched **strand directories** on
startup. This means:

- **Pre-existing files** in a watched strand directory (e.g., strand files
  created by another process before Knot started) will **not** trigger
  `StrandEvent::Created` when the watcher starts.
- **Touching the file** (updating its mtime, e.g. `touch path/to/file.md`)
  is the reliable way to trigger processing of a file that the watcher
  missed. This fires a `Modify` event which Knot processes normally.
- This caveat **does not apply to the event queue**: `events/*.json` files
  (including `Event`-kind deliveries and anything left behind by a crashed
  run) are reloaded at startup (`load_persisted`) and redelivered — no
  touching needed.

## The Event Queue (Both Entry Kinds)

One disk-backed queue per rig (`tie-offs/<rig>/events/*.json`) — the
`.json` files on disk **are** the queue. Knot 0.63.0 (plan 099) retired
the old per-`EventId` dispatch directories (and the per-knot tie-off
files); if an older rig still has them, startup logs `[KNOT][CONFIG]`
deprecation diagnostics and the service never reads them again.

- **Single shared queue across all looms** — events from all knots in all
  looms coexist in the same queue (e.g. `documentation-loom`,
  `review-loom`, `validation-loom` events all queued alongside
  `coding-implementation-loom` events). Each entry carries its own
  `(loom_id, knot_id)` so `ProcessStrand` routes correctly, but there is
  no per-loom queue isolation. The queue is strictly FIFO across all looms.
- **Two entry kinds**: filesystem entries (`Created` / `Modified` /
  `Deleted`) point at a real strand file; `Event` entries carry the
  delivered emission's payload in a `delivery` object (their
  `strand_path` is the synthetic `event:<producer-knot>/<event-id>`
  descriptor, not a path)
- Each entry file is removed from disk **after** its work is done (late
  removal: just before the git commit on success, at the point of
  failure on failure/skip — at-least-once delivery)
- On Knot restart, persisted entry files are reloaded (`load_persisted`)
- `StrandSkipped` entries relate to the filesystem kinds — the file
  referenced by a queued event was missing when processing reached it
  (`Event` entries never skip: the payload is in the entry)

---

## Quick Reference

```bash
# Review recent Knot commits
git log --oneline --grep="knot: " -20

# Review commits with file changes
git log --name-only --grep="knot: " -5

# Review a specific knot's commits
git log --oneline --grep="knot: goals-review " -10

# Query a knot's tie-off history (the unified log — grep, never whole-file read)
grep -n "^## knot=prd-planner " tie-offs/rig/tie-offs.log | tail -10

# See what is currently queued (filesystem + Event-kind entries)
ls -la tie-offs/rig/events/

# Find all event subscriptions (consumer wiring — event: URIs are
# subscription descriptors, not watched paths)
grep -r "strand-dir:.*event:" rig/*-loom/

# Check the service log for dispatched events (the queue fan-out)
grep EventsDispatched tie-offs/rig/knot-service.log

# Follow one emission's durable trace (commit trailers)
git log --grep="knot: " -5 --format="%h %s%n%b" | grep -A4 "^event: "

# Rig source history (separate git repo, user-committed)
git -C rig log --oneline -20

# Review commit diff
git show --stat $(git log --format="%H" --grep="knot: " -1)

# Count commits per knot
git log --oneline --grep="knot: " --format="%s" | \
  sed 's/knot: \([^ ]*\).*/\1/' | sort | uniq -c | sort -rn

# Check for Knot commits in last 24h
git log --oneline --since="24 hours ago" --grep="knot: "

# Trigger processing of a file the watcher missed (touch updates mtime)
touch path/to/strand.md
```

---

## Cross-Reference

**Before using this skill:** Read the `knot-abstractions` skill for the
layered architecture overview (especially the tie-off dispatch mechanism
and producer→consumer interaction model).

Related skills:

1. **knot-abstractions skill** — foundational architecture overview
2. **knot-inspect skill** — inspect current rig state and loom activity
3. **knot-dispatch skill** — trigger knots into action
4. **knot-analyst skill** — analyse live rig productivity and blockers
5. **knot-design skill** — review knot design for authority boundaries and loop patterns
6. **knot-create skill** — create or modify looms, knots, and profiles

This skill (`knot-manage`) provides the **retrospective review** of
completed work. Use knot-inspect for current state and knot-dispatch
for triggering.
