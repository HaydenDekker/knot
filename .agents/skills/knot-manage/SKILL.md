---
name: knot-manage
description: "Review the rig's work using git history and tie-off files. Examine what the rig has produced, assess the quality of agent output, trace the interaction chain between producer and consumer knots to verify communication was effective, and identify gaps between what was triggered and what was delivered. USE FOR: review rig work, check rig output, review tie-offs, review git commits, inspect agent output, check what rig did, review knot work, assess knot quality, review interaction, check communication, review event chain, trace event flow, review dispatch, review producer consumer, review agent comms, check rig productivity, review tie-off quality. DO NOT USE FOR: creating looms (use knot-create), triggering knots (use knot-dispatch), inspecting raw state (use knot-inspect), analysing rig productivity at runtime (use knot-analyst), designing knots (use knot-design)."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.26.0+"
---

# Knot Manage Skill

Review the work produced by a Knot rig. This skill uses git history and
tie-off files to assess what the rig has done, how effectively knots
communicate with each other, and whether the produced output matches
intent.

Unlike `knot-analyst` which monitors **live rig operations** (timeouts,
processing rates, blockers), this skill performs a **retrospective
review** of completed work: what was written, what was communicated,
and whether the interaction chain achieved its goal.

**Tie-off files:** `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`
**Loom-logs:** `rig/tie-offs/{loom-id}/.loom-log`
**Git commits:** Knot-generated commits follow the pattern
`knot: <knot-id> — processed <strand-name> (<event-type>)`

---

## Core Philosophy

### Git Is the Audit Trail

When `git-versioned` is `true` (the default), every successful knot run
creates a git commit. The commit subject identifies the knot and strand;
the body contains the tie-off content. This means `git log` provides a
complete, searchable history of all rig activity — independent of the
rig itself.

### Tie-Offs Are Append-Only History

Each knot's tie-off file grows over time. Every processing event
appends a new section. Reading the tie-off file shows the complete
story of what the knot has done across all its invocations. Sections
are separated by `---` delimiters and each has a header identifying
the knot, event type, strand, and timestamp.

### Interaction Review Is About Signal Quality

When knots communicate via events, the quality of that communication
determines rig effectiveness. This skill reviews:

1. **Was the event emitted?** (producer tie-off contains event block)
2. **Was the event dispatched?** (consumer received a strand in its
   dispatch directory)
3. **Did the consumer react correctly?** (consumer tie-off shows
   meaningful output, not "no changes needed" or errors)
4. **Is the event payload sufficient?** (consumer had enough facts
   to do its job without instructions)

---

## Prerequisites

1. The project must be in a git repository with Knot-generated commits.
   Verify with: `git log --oneline --grep="knot: " | head -10`
   If no Knot commits exist, either `git-versioned` is `false` on
   knots or the rig has not produced successful output yet.
2. Tie-off files must exist. Check `rig/tie-offs/` for content.

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
   | Commits only touch `rig/tie-offs/` | Knots are producing tie-offs but not writing to their target domains. Check knot instructions — they may not be told where to write output. |
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

1. **Read the tie-off file**:
   `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`

2. **Parse the sections**: Each section has:
   ```
   ## {knot-name} triggered by {event-type} {strand-path}
   Timestamp: {iso8601}
   ---
   Agent output...
   ```

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
   Verify that tie-off entries correspond to actual commits (the
   tie-off may have entries from before `git-versioned` was enabled,
   or from failed runs that didn't commit).

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

### Tie-off Events vs. Dispatched Event Strands

When reviewing interaction chains, distinguish between two different
artifacts — confusing them is a common source of false "dead
subscription" diagnoses:

**Event blocks in the producer's tie-off body** (`tie-off-{knot-name}.md`):
These are ```markdown fenced code blocks the producer agent writes as
part of its output. They *declare* that an event occurred. Knot parses
these blocks **after** the producer completes and dispatches matching
events to consumers. If the producer writes `event: None` or omits
an expected event, nothing is dispatched — the producer agent simply
didn't emit it (either no event was warranted, or the agent was not
instructed to emit that event type).

**Dispatched event strand files** (`{event-id}/` subdirectory):
These are separate `.md` files created by Knot's event parser — **not**
by the producer agent. They appear in the consumer's dispatch directory
and are what the consumer knot actually processes as strands. If the
directory exists but is empty (only a `DirectoryCreated` loom-log entry),
it means Knot created the watch directory for the subscription but no
event has been dispatched yet.

The producer **never writes to the dispatch directory directly**.
The flow is:

1. Producer agent writes event block in its tie-off body
2. Knot parses the tie-off after agent completes
3. Knot creates a dispatched strand file in the consumer's dispatch directory
4. Consumer watches that directory and processes the new strand file

If step 1 never happened, steps 2–4 never occur.

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

2. **Trace the producer's tie-off**:
   Read `rig/tie-offs/{producer-loom-id}/tie-off-{producer-knot-name}.md`

   For each section that contains an event block (a ```markdown code
   block with `event: <EventId>`):
   - Record the event ID, payload fields, and body text.
   - Note which strand triggered this processing event.

3. **Trace the consumer's tie-off**:
   Read `rig/tie-offs/{consumer-loom-id}/tie-off-{consumer-knot-name}.md`

   For each section:
   - Check the header's strand path — it should point to an event
     file in `rig/tie-offs/{consumer-loom-id}/{EventId}/`.
   - Correlate the timestamp with the producer's event emission.
   - Assess the consumer's output: did it react meaningfully to
     the event?

4. **Check the dispatch directory**:
   List `rig/tie-offs/{consumer-loom-id}/{EventId}/` to see all
   dispatched event files. Each file corresponds to one producer
   emission.

   ```bash
   ls -la rig/tie-offs/{consumer-loom-id}/{EventId}/
   ```

5. **Assess interaction quality**:

   | Dimension | Good | Poor |
   |-----------|------|------|
   | **Event emission** | Producer emits events when warranted, with factual payload | Producer emits `event: None` when something significant happened, or emits events for trivial changes |
   | **Event payload** | Payload contains facts the consumer needs (identifiers, values, descriptions) | Payload is empty or contains only a timestamp — consumer must guess context |
   | **Event instructions** | Payload provides context, not commands | Payload contains "you should do X" — the event has absorbed the consumer's responsibility |
   | **Consumer reaction** | Consumer reads event, investigates its domain, makes targeted changes | Consumer reports "no changes needed" for every event, or makes broad unrelated changes |
   | **Timeliness** | Consumer processes event shortly after dispatch | Large gaps between producer emission and consumer processing |
   | **Completeness** | All producer events have corresponding consumer processing | Some events dispatched but consumer tie-off has no matching entry |

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
   | **Unanswered events** | Producer emitted events (in tie-off) but consumer's tie-off has no corresponding entries. Check consumer's loom-log for `KnotProcessing` on those event strands. |
   | **Dead subscriptions** | Consumer subscribes to an event but the producer's tie-off never emits it. The dispatch directory is empty. |
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

## Tie-Off File Format

Tie-off files are append-only markdown. Each section has a standard
header followed by the agent's output:

```markdown
## {knot-name} triggered by {event-type} {strand-path}
Timestamp: 2026-07-24T12:00:00Z
event: PlanCreated          ← present if this was triggered by an event
source: producer-knot       ← present if event-triggered
original_strand: ...        ← original strand that caused the event
---
Agent output for this processing event...

```markdown
---
event: SomeEvent
description: ...
---
Event context
```
---
## {knot-name} triggered by Modified other-strand.md
Timestamp: 2026-07-24T13:00:00Z
---
Agent output for the second event...
```

**Section header fields:**

| Field | Description |
|-------|-------------|
| `knot-name` | The knot's identifier |
| `event-type` | `Created`, `Modified`, or `Deleted` (the strand event that triggered processing) |
| `strand-path` | Path of the strand file processed |
| `Timestamp` | ISO 8601 UTC timestamp of processing |
| `event` | Present when triggered by an event strand — shows the event ID |
| `source` | Present when event-triggered — shows the producing knot's name |
| `original_strand` | Present when event-triggered — shows the original strand that caused the upstream event |

**Event blocks within tie-off body:**

When a knot is a producer, its tie-off body may contain ```markdown
code blocks with event frontmatter. These are parsed by Knot and
dispatched to consumers.

---

## Error Handling

| Scenario | Action |
|----------|--------|
| No git repository | Project may not be in git. Skip git analysis, review tie-offs only. Note in report. |
| No Knot commits | Either `git-versioned: false` on all knots, or the rig has not produced successful output. Check tie-off files directly. |
| Tie-off file missing | Knot has not yet produced output. Not an error — knot may not have been triggered. |
| Tie-off file is empty | Knot may have started processing but not completed. Check loom-log for `KnotProcessing` without `KnotCompleted`. |
| Git user not configured | Commits may fail silently. Check `git config user.email` and `git config user.name`. |

## Loom-Log Entries to Ignore During Review

When reviewing loom-logs (`rig/tie-offs/{loom-id}/.loom-log`), these entries
are expected and do not indicate problems:

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
  The event is persisted in the queue (`rig/events/*.json`), and when
  `ProcessStrand` pops it, the file is already gone. The event file is
  auto-removed from the queue on pop, so this does not recur from the same
  event. If you see many of these for the same path, investigate what is
  creating and deleting files in the strand directory.

- **`StrandIgnored` with reason `"binary file"`** — A binary file appeared in
  a strand directory. Knot skips non-text files to avoid passing binary data
  to the agent. Not an error unless binary files should not be in the strand
  directory at all (e.g. a build artifact leaked in).

## File Watcher and Pre-Existing Files

The `notify` file watcher only fires events for changes that occur *after*
`watch()` is called. It does not scan the watched directory on startup.
This means:

- **Pre-existing files** in a watched directory (e.g., strand files created
  by another process before Knot started, or event files left from a
  previous run) will **not** trigger `StrandEvent::Created` when the
  watcher starts.
- **Touching the file** (updating its mtime, e.g. `touch path/to/file.md`)
  is the reliable way to trigger processing of a file that the watcher
  missed. This fires a `Modify` event which Knot processes normally.
- This commonly affects **event dispatch directories**:
  `rig/tie-offs/{loom-id}/{EventId}/` — if the directory was created in
  a prior run and already contains event files, restarting Knot will start
  a new watcher but won't retroactively process the existing files.
  Touch any unprocessed event files to trigger them.

## Event Queue vs. Dispatch Directories

These are two separate mechanisms — confusing them is a common source of
misdiagnosis:

**Event queue** (`rig/events/*.json`):
- Disk-backed — the `.json` files on disk **are** the queue
- **Single shared queue across all looms** — events from all knots in all
  looms coexist in the same queue (e.g., `documentation-loom`,
  `review-loom`, `validation-loom` events all queued alongside
  `coding-implementation-loom` events). Each event carries its own
  `(loom_id, knot_id)` so `ProcessStrand` routes correctly, but there is
  no per-loom queue isolation. The queue is strictly FIFO across all looms.
- Holds pending filesystem change events (`Created`, `Modified`, `Deleted`)
  triggered by the file watcher watching strand directories
- Each event file is removed from disk when popped for processing
- On Knot restart, persisted event files are reloaded (`load_persisted`)
- `StrandSkipped` entries relate to this queue — the file referenced by a
  queued event was missing when processing reached it

**Dispatch directories** (`rig/tie-offs/{loom-id}/{EventId}/`):
- Hold event strand files created by Knot's event dispatcher
- Used for producer→consumer intent-based routing
- Each `.md` file is a one-shot event with YAML frontmatter and body
- The consumer knot watches these directories as its `strand-dir`
- These are **not** the event queue — they are regular strand files that
  the consumer knot processes normally

---

## Quick Reference

```bash
# Review recent Knot commits
git log --oneline --grep="knot: " -20

# Review commits with file changes
git log --name-only --grep="knot: " -5

# Review a specific knot's commits
git log --oneline --grep="knot: goals-review " -10

# Read a knot's tie-off history
cat rig/tie-offs/planning-loom/tie-off-prd-planner.md

# Check event dispatch directory
ls -la rig/tie-offs/planning-loom/ReviewCompleted/

# Find all event subscriptions (consumer wiring)
grep -r "strand-dir:.*event:" rig/*-loom/

# Check loom-log for dispatched events
grep EventsDispatched rig/tie-offs/review-loom/.loom-log

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
