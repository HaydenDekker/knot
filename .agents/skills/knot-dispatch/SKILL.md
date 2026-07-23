---
name: knot-dispatch
description: "Trigger knots into action by creating or touching strand files, dispatching events manually, and understanding the event flow from producer to consumer. Covers filesystem triggers (creating/touching files in strand-dir), event file creation in dispatch directories, verifying triggers via loom-logs and state, and the full dispatch pipeline from strand creation to tie-off completion. USE FOR: trigger knot, dispatch knot, fire knot, add strand, touch strand, trigger event, dispatch event, manual trigger, strand trigger, start processing, fire agent, activate knot, knot trigger, event trigger, strand creation, event file, trigger dispatch. DO NOT USE FOR: creating looms (use knot-create), inspecting state (use knot-inspect), designing knots (use knot-design), analysing rig productivity (use knot-analyst)."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.26.0+"
---

# Knot Dispatch Skill

Trigger knots into action by adding or touching strand files, manually
creating event files, and understanding the full dispatch pipeline from
strand creation through tie-off completion.

Knots are event-driven — they react to filesystem changes in their
`strand-dir`. This skill covers how to trigger them, verify they fired,
and troubleshoot when they don't.

**State file:** `rig/state.json` (written every 5 seconds by Knot)
**Activity logs:** `rig/tie-offs/{loom-id}/.loom-log` (append-only JSONL)
**Tie-off output:** `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`

---

## Core Philosophy

### Everything Is a Strand Event

Knots have one input direction (`strand-dir`). Whether it points to a
filesystem directory or an `event:` URI, the trigger is always the same:
a file appears or changes in the watched directory. There is no "start"
command — you dispatch work by touching the filesystem.

### File Watch, Not Polling

Knot uses `notify` (inotify on Linux, FSEvents on macOS) for instant
delivery. When a file is created, modified, or deleted in a watched
directory, Knot receives the event within milliseconds. No polling loop
is needed.

### Debounce Protects Against Rapid Changes

If multiple changes occur to the same file within the debounce window
(500ms default), Knot coalesces them into a single `Modified` event.
This prevents re-processing mid-edit. The final processed state
reflects the last saved content.

---

## Prerequisites

1. Knot must be running and the rig must be initialised.
   Verify by checking `rig/state.json` exists and has fresh `updated_at`.
   If not, use the `knot-init` skill.
2. At least one loom with at least one knot must exist.
   Check `rig/state.json` `looms` array. If empty, use `knot-create`
   to create looms and knots first.

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
Tie-off appended to rig/tie-offs/{loom-id}/tie-off-{knot-name}.md
    ↓
Events in tie-off parsed → dispatched to consumer knots (fan-out)
```

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

1. **Identify the strand directory**: Read `rig/state.json` and find
   the target knot. Check its `strand-dir` field.

2. **Verify the knot is registered**: Find the knot in `rig/state.json`
   `looms[].knots[]` array. Check its `status` is not `processing`
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

4. **Wait for processing**: The debounce window is 500ms. Processing
   typically completes within seconds to minutes depending on the
   agent's work.

5. **Verify the knot fired**: Read `rig/tie-offs/{loom-id}/.loom-log`
   and look for `KnotProcessing`, then `KnotCompleted` (or
   `KnotFailed`) for the target knot.

6. **Check the tie-off output**: Read
   `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md` and verify the
   latest section contains the expected output.

### Trigger an Event Consumer Knot

When asked to trigger a knot whose `strand-dir` is an `event:` URI
(e.g. `event:quality-reviewer:ReviewCompleted`):

The consumer knot watches a dispatch directory at
`rig/tie-offs/{consumer-loom-id}/{EventId}/`. To trigger it manually:

1. **Identify the dispatch directory**: Read the consumer knot's
   definition file at `rig/{loom-id}/{knot-name}.md`. Extract the
   `strand-dir` event URI to determine the `EventId`.

   The dispatch directory is:
   `rig/tie-offs/{consumer-loom-id}/{EventId}/`

2. **Create an event file** in the dispatch directory:
   ```bash
   mkdir -p rig/tie-offs/{consumer-loom-id}/{EventId}
   cat > rig/tie-offs/{consumer-loom-id}/{EventId}/event-manual.md << 'EOF'
   ---
   event-id: {EventId}
   target-knot: manual-trigger
   timestamp: 2026-07-24T12:00:00Z
   ---

   ## Event: {EventId} from manual-trigger

   Manual trigger for testing.
   EOF
   ```

   The file must have YAML frontmatter with at least `event-id` to be
   recognised. The frontmatter fields `event-id`, `target-knot`, and
   `timestamp` are standard — add any additional payload fields as
   needed.

3. **Wait for processing** — the consumer knot should pick up the new
   event file as a strand event.

4. **Verify via loom-log**: Read `rig/tie-offs/{consumer-loom-id}/.loom-log`
   for `KnotProcessing` and `KnotCompleted` entries.

5. **Clean up** (optional): Remove the manual event file after
   processing if it was a one-off test:
   ```bash
   rm rig/tie-offs/{consumer-loom-id}/{EventId}/event-manual.md
   ```

### Trigger a Producer Knot (Which Emits Events to Consumers)

When a producer knot completes, Knot scans its tie-off for event blocks
and dispatches matching events to consumer knots. To trigger the
full producer→consumer chain:

1. **Trigger the producer** using the normal filesystem trigger above
   (create/touch a file in the producer's `strand-dir`).

2. **Wait for the producer to complete**: Monitor its loom-log for
   `KnotCompleted`.

3. **Check for event dispatch**: Look for `EventsDispatched` entries
   in the producer's loom-log. These show which events were emitted
   and which consumer looms received them.

4. **Verify consumer activation**: Read each consumer loom's log for
   `KnotProcessing` entries that follow the dispatch.

5. **Trace the full chain**: The sequence should be:
   ```
   Producer: KnotProcessing → KnotCompleted → EventsDispatched
   Consumer: KnotProcessing → KnotCompleted
   ```

### Verify a Trigger Worked

After triggering any knot:

1. **Check `rig/state.json`**: The knot's `status` should be
   `completed` (or `failed` if something went wrong).
   The `last_event_at` field should have a recent timestamp.
   The `last_strand_path` should point to the file you created/touched.

2. **Check the loom-log**: Tail
   `rig/tie-offs/{loom-id}/.loom-log` and look for:
   ```json
   {"KnotProcessing": {"knot_id": "...", "strand_path": "...", ...}}
   {"KnotCompleted": {"knot_id": "...", "strand_path": "...", "tie_off_path": "...", ...}}
   ```

3. **Check the tie-off**: Read
   `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`. A new section
   should appear at the end of the file with a header like:
   ```
   ## {knot-name} triggered by Created {strand-filename}
   Timestamp: 2026-07-24T12:00:00Z
   ---
   Agent output here...
   ```

---

## Troubleshooting

### Knot Did Not Fire

| Symptom | Check |
|---------|-------|
| No `KnotProcessing` in loom-log | Verify Knot is running (`rig/state.json` `updated_at` is fresh). Check `rig/.rig-log` for errors. |
| `KnotProcessing` but no `KnotCompleted` | Agent may have timed out. Check `rig/.rig-log` for `TimeoutExceeded`. Increase profile `timeout`. |
| `KnotFailed` in loom-log | Read the error message in the log entry. Common causes: missing profile, parse errors, agent crash. |
| File created but no event at all | The strand directory may not be watched. Check `rig/state.json` — is the loom and knot registered? Knot may need a restart to pick up new watches. |
| Event file created but consumer did not fire | Verify the event file has valid YAML frontmatter with `event-id`. Check the dispatch directory path matches the consumer's `strand-dir` event URI. |

### Triggering Too Frequently

If the same strand keeps re-triggering the same knot:

- **Debounce**: Rapid edits are coalesced. Only one `Modified` event
  fires after the debounce window closes (500ms after last change).
- **Idempotency**: Knots should be idempotent (see `knot-design`
  skill). Re-triggering on the same strand should produce
  "no changes needed" after the first run.
- **Strand pinning**: If you need to prevent re-processing, the
  strand file should not change after the initial trigger.

### Event Not Reaching Consumer

| Symptom | Check |
|---------|-------|
| Producer emitted event but consumer idle | Check `EventsDispatched` in producer's loom-log — does it list the consumer's loom? If not, the consumer's `strand-dir` event URI may not match the producer's knot name. |
| Event file exists in dispatch dir but consumer idle | Consumer's dispatch directory may not be watched. Restart Knot or check loom-log for `KnotRegistered` for the consumer. |
| `event: None` in tie-off but expected event | The producer agent was instructed to emit an event but chose `None`. Check the producer's tie-off content — it may have decided no event was warranted. |
| Multiple consumers, only some fired | Each consumer matches independently. Check each consumer's `strand-dir` event URI against the producer's knot ID. |

---

## Event File Format

When creating event files manually (for testing or external triggers),
use the standard event file format that matches what Knot generates:

```markdown
---
event-id: PlanCreated
target-knot: plan-creator
timestamp: 2026-07-24T12:00:00Z
plan: PLAN-001
description: New implementation plan
---

## Event: PlanCreated from plan-creator

A new plan was created covering three phases.
```

**Required frontmatter fields:**

| Field | Required | Description |
|-------|----------|-------------|
| `event-id` | **Yes** | Event identifier (must match consumer's subscription) |
| `target-knot` | **Yes** | Name of the producing knot (or `manual-trigger` for external events) |
| `timestamp` | **Yes** | ISO 8601 UTC timestamp |

**Optional frontmatter fields:**

Any additional `key: value` pairs become the event payload. The
consumer knot receives these as part of the strand file it reads.

---

## Producer Event Emission

When a knot has consumers listening for its events, Knot injects event
instructions at the **beginning** of its prompt. The injected block
tells the agent which events it may emit and the required format:

```
## Agent Events

Other knots are listening for events you may emit.

Events you may emit:
- `ReviewCompleted` — Emitted when a quality review is complete.

If an event occurred, emit in your output:
```markdown
---
event: ReviewCompleted
description: <short summary of what happened>
<additional fields as relevant>
---

<Narrative context>
```

If no events occurred, emit:
```markdown
---
event: None
---
```
```

The producer writes this block in its tie-off. Knot parses the
```markdown code block, extracts the event, and dispatches it to
matching consumers. The producer does **not** need to declare its
events — Knot discovers them from consumer subscriptions.

**Multiple events** can be emitted in one tie-off — each as a separate
```markdown block. Each event is dispatched independently.

---

## Quick Reference

```bash
# Trigger a normal knot (filesystem strand-dir)
echo "work item" > project/prds/new-feature.md

# Trigger an event consumer (manual event file)
mkdir -p rig/tie-offs/planning-loom/ReviewCompleted
cat > rig/tie-offs/planning-loom/ReviewCompleted/event-manual.md << 'EOF'
---
event-id: ReviewCompleted
target-knot: quality-reviewer
timestamp: 2026-07-24T12:00:00Z
---

## Event: ReviewCompleted from quality-reviewer

Manual review trigger.
EOF

# Verify trigger worked — check loom-log
tail -5 rig/tie-offs/planning-loom/.loom-log

# Verify trigger worked — check state
cat rig/state.json | python3 -c "
import sys, json
state = json.load(sys.stdin)
for loom in state['looms']:
  for knot in loom['knots']:
    print(f\"{loom['id']}/{knot['id']}: {knot['status']} (last: {knot.get('last_event_at', 'never')})\")"

# Check tie-off output
cat rig/tie-offs/planning-loom/tie-off-refactor-planner.md | tail -20

# Check event dispatch in producer's loom-log
grep EventsDispatched rig/tie-offs/review-loom/.loom-log
```

---

## Cross-Reference

Related skills:

1. **knot-create skill** — create looms, knots, and profiles (must exist before triggering)
2. **knot-inspect skill** — inspect rig state and verify trigger results
3. **knot-design skill** — design knots with correct event contracts
4. **knot-manage skill** — review the work produced by triggered knots

This skill covers **triggering** — getting knots to run. The other
skills handle setup, monitoring, and design.
