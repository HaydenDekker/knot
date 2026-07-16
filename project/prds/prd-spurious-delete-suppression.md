# PRD: Spurious Delete Suppression — Burst Event Deduplication

## Problem

External tooling that rewrites files using an atomic save pattern (truncate + write) fires two inotify events: `DELETE` followed by `CREATE` or `MODIFY` for the same file. These events can arrive up to a second apart, exceeding Knot's 100 ms debounce window.

When this burst occurs — commonly from editors with atomic save, git stash/checkout, or AI tool use — Knot processes the DELETE event as a legitimate strand deletion, invoking the agent pipeline to handle a file as "deleted" when it was actually just rewritten. This wastes agent invocations and produces misleading tie-off entries that record deletion processing for files that still exist.

The user cannot distinguish between a file that was genuinely deleted and one that was momentarily truncated and rewritten. Every spurious DELETE triggers a full agent invocation with deletion semantics, consuming time and provider tokens.

Furthermore, a move event could occur, realised as delete in one folder and a create in another location, and a bulk delete event could occur, many files from multiple directories at a signle time. 

## Goals

- [ ] Spurious DELETE events (DELETE followed by CREATE/MODIFY within a short window) are suppressed — the agent pipeline is never invoked for the transient deletion
- [ ] Legitimate deletions (file actually removed with no subsequent creation) are still processed normally after the suppression window expires
- [ ] Suppressed events are observable in the rig-log for diagnosis
- [ ] The existing 100 ms per-file debounce window for repeated same-kind events is preserved unchanged
- [ ] The suppression window is configurable via rig configuration (default 5 seconds)

## Non-Goals

- Generalising the debounce window for all event types
- Git-based file change verification
- Handling burst events at the inotify watcher level
- Suppressing events caused by Knot's own file writes (Knot does not rewrite strand files)

## User Stories

### Story 1: Atomic Save Does Not Trigger Deletion Processing

As a user editing files with an editor that uses atomic save (VS Code, vim swap), I want Knot to recognise that a file was rewritten — not deleted — so that the agent pipeline is not invoked for a transient deletion.

**Scenarios:**
1. Given a file `doc.md` exists in a watched strand directory, when the editor truncates and rewrites `doc.md` (firing DELETE then MODIFY 1 second apart), then Knot processes only the MODIFY event — not the DELETE
2. Given a burst of 10 files rewritten atomically, when all files fire DELETE then MODIFY within 5 seconds, then Knot processes 10 MODIFY events and 0 DELETE events

### Story 2: Legitimate Deletions Are Still Processed

As a user deleting files from a watched strand directory, I want Knot to process the deletion so that the agent handles the removal with appropriate semantics (e.g. archiving context from previous tie-offs).

**Scenarios:**
1. Given a file `doc.md` exists in a watched strand directory, when I permanently delete `doc.md` (no subsequent CREATE or MODIFY), then Knot processes the DELETE event after the suppression window (5 seconds) expires
2. Given a file is deleted and recreated with a completely different name, when no CREATE/MODIFY for the *same path* arrives within 5 seconds, then Knot processes the original DELETE event

### Story 3: Observability of Suppressed Events

As a user debugging unexpected pipeline behaviour, I want to see which DELETE events were suppressed so I can understand why the agent was not invoked for a deletion I expected.

**Scenarios:**
1. Given a DELETE event was suppressed because a MODIFY arrived within the window, when I inspect the rig-log, then I see a log entry recording the suppression with the file path, loom, knot, and the event that caused the suppression

## Success Criteria

- [ ] Burst events (13 DELETE + 13 MODIFY, 1 second apart) result in exactly 13 MODIFY events reaching the agent pipeline and 0 DELETE events
- [ ] A single genuine DELETE (no subsequent CREATE/MODIFY within 5 seconds) results in exactly 1 DELETE event reaching the pipeline after the window
- [ ] Each suppressed DELETE is logged to the rig-log with path, loom, knot, and suppression reason
- [ ] The existing 100 ms debounce window for same-kind events is unchanged and passes all existing tests
- [ ] Suppression window is configurable via rig configuration with a sensible default (5 seconds)

## Dependencies & Constraints

- Must not extend or change the existing 100 ms same-kind debounce window
- The suppression window must be long enough to cover the truncate+rewrite pattern (observed gap: ~1 second) but not so long that legitimate deletions are delayed unacceptably
- Implementation must not use subprocess calls (e.g. git diff) to verify file state — this adds unnecessary overhead and git may not be available

## Implementation Status: 🔵 Open
