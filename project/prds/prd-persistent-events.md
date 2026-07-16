# PRD: Persistent Events — Disk-Backed Event Queue

## Problem

Knot currently queues strand events in-memory. When the process stops (Ctrl+C, crash, or restart), all pending events are lost. If a user stops Knot with events queued — for example, a burst of file changes triggered several strand events but the agent hasn't processed them all yet — restarting Knot silently discards those unprocessed events. The user has no visibility into what was pending and no way to recover the lost work.

Additionally, while events are queued, the user has no way to inspect, reorder, or cancel pending work. The queue is an opaque in-memory structure — the user cannot see "what's next" or decide "I don't want that strand processed right now."

Knot needs a **persistent event queue** so that pending strand events survive restarts, and the user has control over the queued work.

## Goals

- [ ] Pending strand events are written to the file system in `rig/events/` so they survive process restarts
- [ ] On startup, Knot scans `rig/events/` and re-initialises the in-memory queue with any persisted events before starting normal processing
- [ ] When an event is popped from the queue for processing, its file is removed from `rig/events/` so it is not re-processed on the next restart
- [ ] A user can list pending events via the HTTP interface to see what is queued and in what order
- [ ] A user can delete a pending event via the HTTP interface to cancel processing for that strand
- [ ] A user can modify a pending event's file on disk to alter its properties before processing (e.g. change the event type)
- [ ] Events in `rig/events/` are stored as individual JSON files so the user can inspect them with standard tools (cat, ls, etc.)
- [ ] The existing in-memory `InspectQueue` behaviour (debounce dedup, FIFO ordering) is preserved — persistence is additive, not a replacement

## Non-Goals

- Event priority or reordering via the API (users can delete files manually if they need to change order)
- Distributed event queues — Knot is a single-machine service
- Event retry/backoff policies — that belongs in the System Reliability PRD
- Schema migration of persisted events — if the event format changes, old files are skipped with a warning
- HTTP-based event creation — events are created by the file-watcher pipeline and by the restart-scan, not by external HTTP calls
- Persistence for the debounce engine's internal pending map — only the output queue is persisted

## User Stories

### Story 1: Survive Restart

As a user, when I stop Knot while events are queued, I expect Knot to continue processing those events after I restart it, so that no work is silently lost.

**Scenarios:**

1. Given 5 strand events are queued but only 2 have been processed, when I stop Knot and restart it, then the remaining 3 events are re-queued and processed automatically
2. Given no events are queued when I stop Knot, when I restart it, then Knot starts cleanly with an empty queue
3. Given events exist in `rig/events/` from a previous session, when Knot starts, then those events are loaded into the queue before normal file-watcher events begin arriving
4. Given an event file in `rig/events/` is malformed (invalid JSON), when Knot starts, then the file is skipped with a warning logged to stderr and processing continues with the remaining valid events

### Story 2: Inspect Pending Events

As a user, I want to see what events are currently queued so I can understand what work is pending and in what order.

**Scenarios:**

1. Given there are 3 events in the queue, when I query the HTTP interface for pending events, then I see all 3 events with their type (Created/Modified/Deleted), strand path, loom ID, and knot ID
2. Given the queue is empty, when I query the HTTP interface for pending events, then I see an empty list
3. Given events are being processed (one is in-flight while 2 are queued), when I query pending events, then I see the 2 queued events — the in-flight event is not shown as pending

### Story 3: Delete Pending Events

As a user, I want to cancel a pending event so that it is not processed.

**Scenarios:**

1. Given 3 events are queued, when I delete one via the HTTP interface, then it is removed from the queue and its file is removed from `rig/events/`, and the remaining 2 events continue normally
2. Given I delete the last event in the queue, when I check the queue, then it is empty and Knot writes a `QueueIdle` entry to the rig-log
3. Given an event file is deleted from `rig/events/` on disk while Knot is running, when Knot next processes the queue, then the deleted event is not processed (the file is simply absent)

### Story 4: Modify Pending Events

As a user, I want to edit the properties of a pending event before it is processed, so I can adjust the event type or metadata if needed.

**Scenarios:**

1. Given an event file exists in `rig/events/` representing a `Modified` event, when I edit the file on disk to change it to a `Created` event, when Knot processes the queue, then the event is processed with the updated type
2. Given I modify an event file on disk while Knot is running, when Knot next pops that event from the queue, then it reads the updated file content (the disk file is the source of truth for persisted events)

### Story 5: File-First Event Store

As a user, I want events to be stored as individual files I can inspect with standard tools, so I don't need a special client to understand what's queued.

**Scenarios:**

1. Given events are queued, when I `ls rig/events/`, then I see one JSON file per pending event
2. Given an event file exists in `rig/events/`, when I `cat` the file, then I see the event data in readable JSON format (strand path, loom ID, knot ID, event type, and timestamp)
3. Given an event has been processed, when I `ls rig/events/`, then the file for that event is no longer present

## Success Criteria

- [ ] Events written to `rig/events/<unique-id>.json` on queue push, removed on pop
- [ ] On startup, `rig/events/` is scanned and valid JSON event files are loaded into the in-memory queue before processing begins
- [ ] Malformed event files are skipped with a warning — they do not crash the startup
- [ ] The HTTP interface exposes a `GET` endpoint to list all pending events (reads from the queue, not the disk)
- [ ] The HTTP interface exposes a `DELETE` endpoint to remove a specific pending event by ID
- [ ] Deleting an event removes it from both the in-memory queue and the `rig/events/` directory
- [ ] The existing `InspectQueue` dedup and FIFO behaviour is preserved
- [ ] The `QueueIdle` detection in the process-strand loop still works correctly (idle = no events in queue AND no events in `rig/events/`)
- [ ] All existing tests pass — the change is backwards compatible

## Dependencies & Constraints

- **Technical dependency:** The `InspectQueue` must gain a persistence layer. The current `push()` / `pop()` methods need to coordinate with file I/O. This could be a wrapper or an adapter.
- **Technical constraint:** Event files must be written atomically (write to temp, rename) to prevent partial files from being read on startup.
- **Technical constraint:** The unique file name must encode enough information for deduplication — if the same event is re-queued (e.g. restart while the debounce engine is also running), duplicates must be collapsed by the existing `push_or_replace` mechanism.
- **Design decision:** Events are stored as JSON files, not YAML or `.md`. JSON is the natural format since `StrandEvent` already derives `Serialize + Deserialize`. The files are machine-readable and tool-inspectable.
- **Design decision:** `rig/events/` is a flat directory — no subdirectories. Each file is named with a unique ID (e.g. a ULID or timestamp-based name) to guarantee uniqueness and FIFO ordering by filename sort.
- **Design decision:** The in-memory `InspectQueue` remains the primary processing queue. Persistence is an add-on — events are written to disk when pushed and removed when popped. On startup, disk events are loaded into the in-memory queue. This keeps the existing pipeline architecture intact.
- **Design decision:** When a user modifies an event file on disk while Knot is running, the in-memory queue still holds the original value. The modified file is read fresh only when the event is popped (if the persistence layer reads-from-disk on pop) OR the user must delete and re-create the file. The simplest approach: the HTTP DELETE removes both the in-memory entry and the disk file; the user can edit the file on disk and Knot reads it on next startup.
- **Technical constraint:** The startup scan must complete before the file-watcher begins emitting new events, otherwise new events could arrive before old persisted events are loaded, breaking FIFO ordering across restart boundaries.
- **Configuration constraint:** The events directory path (`rig/events/`) is fixed — not user-configurable. It lives at the rig root alongside `state.json`, `.rig-log`, `profiles/`, and `tie-offs/`.

## Implementation Status: 🔵 Open
