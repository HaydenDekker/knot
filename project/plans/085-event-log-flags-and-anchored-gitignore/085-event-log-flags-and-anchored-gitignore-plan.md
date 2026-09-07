# Plan: Event-Parse Log Flags and Anchored Rig `.gitignore` Entry

## Implementation Status: 📋 Planned

## Problem

Two small quality-of-life defects surfaced during rig operation:

1. **Event-parse console line hides event status.** When a knot completes and
   Knot extracts the structured agent events from its tie-off, it logs every
   event it found:
   ```
   event parse (knot=author): 2 event(s) found — ["PlanCreated", "SpecReviewed"]
   ```
   This lists *all* events (occurred and non-occurred alike) but discards the
   `occurred` boolean that distinguishes a **real event** (`true`, dispatched to
   consumers) from an **acknowledgement** (`false`, counted for enforcement but
   never dispatched). From the console line alone it is impossible to tell which
   events actually fired. Operators want to see the boolean alongside each id.

2. **Rig `.gitignore` entry is unanchored and over-matches.** On knot start,
   `ensure_rig_repo()` appends the rig directory to the parent repo's
   `.gitignore` as an *unanchored* pattern `rig/`. Git treats a slash-less
   pattern as matching at **any depth**, so `rig/` also ignores
   `tie-offs/rig/` — a nested directory that the user wants **versioned**. The
   entry must be anchored to the repository root with a leading slash
   (`/rig/`) so it matches only the top-level rig directory.

## Target

1. **Event-parse line shows the flag.** The console diagnostic becomes
   `... — PlanCreated=true, SpecReviewed=false`. This is a stderr diagnostics
   change only — dispatch behaviour (which already filters on `occurred`) is
   unchanged.

2. **Rig exclusion is root-anchored.** The appended `.gitignore` entry becomes
   `/{basename}/` (e.g. `/rig/`, `/dev-rig/`). `tie-offs/rig/` is no longer
   ignored by the rig-exclusion rule.

3. **Force-migrate existing entries.** Rigs initialised by older binaries
   already carry the unanchored `rig/` line. On startup, `ensure_rig_repo()`
   rewrites any existing bare `{basename}/` line to `/{basename}/` in place
   (preserving surrounding content and the marker), rather than appending a
   duplicate. The change is idempotent — an already-anchored `/rig/` line is
   left untouched.

## Existing Tests

| Test | What it covers | Status |
|---|---|---|
| `git_versioner.rs::ensure_rig_repo_appends_marked_entry` | Appends marked entry, preserves existing content | ✅ Green — **needs update to `/rig/`** |
| `git_versioner.rs::ensure_rig_repo_is_idempotent` | Entry + marker appended exactly once | ✅ Green — **needs update to `/rig/`** |
| `git_versioner.rs::ensure_rig_repo_uses_rig_basename_for_named_rigs` | Named rig excluded by basename | ✅ Green — **needs update to `/dev-rig/`** |
| `git_versioner.rs::ensure_rig_repo_*` (no-git / tracked cases) | No edit when parent is not a repo / rig is tracked | ✅ Green |
| `process_strand.rs` event-dispatch tests | Events with `occurred: false` produce no dispatch; `EventsDispatched` logged correctly | ✅ Green (behaviour unchanged) |

## Test Gaps

- No test asserts the event-parse line includes the `occurred` flag (it is
  `eprintln!` diagnostics — capture is awkward; covered by inspection, no new
  unit test required unless a log-port seam is introduced).
- No test for **force-migrating** a pre-existing bare `rig/` line to `/rig/`.
- No test that an **already-anchored** `/rig/` line is left as-is (idempotent).

## Phases

### Phase 1: Show `occurred` flag on the event-parse line

**File:** `src/application/usecases/process_strand.rs` → `dispatch_agent_events()`
(~line 700)

Replace the id-only summary with a per-event `id=bool` summary:

```rust
// Before
let all_event_ids: Vec<&str> = all_events.iter().map(|e| e.event_id.as_str()).collect();
eprintln!(
    "event parse (knot={}): {} event(s) found — {:?}",
    knot.id.0, total_count, all_event_ids,
);

// After
let summary: Vec<String> = all_events.iter()
    .map(|e| format!("{}={}", e.event_id, e.occurred))
    .collect();
eprintln!(
    "event parse (knot={}): {} event(s) found — {}",
    knot.id.0, total_count, summary.join(", "),
);
```

The dispatch filter below (`filter(|e| e.occurred)`) is untouched — this is a
pure diagnostics improvement.

**Tests:** none added (stderr diagnostics). Manually confirm the format in a
live run elsewhere; unit behaviour unchanged so existing dispatch tests stay green.

### Phase 2: Anchor and force-migrate the rig `.gitignore` entry

**File:** `src/adapters/outbound/git_versioner.rs` → `ensure_rig_repo()`

1. **Anchor the entry** (~line 341):
   ```rust
   let entry = format!("/{basename}/");   // was: format!("{basename}/")
   ```

2. **Force-migrate on the exclusion check** (~line 347). The current check
   treats a line as "already excluded" only when it equals `entry` (now
   `/rig/`) or the marker. Extend it so that:
   - An existing **bare** `{basename}/` line is **rewritten in place** to
     `/{basename}/` (force migration), preserving the marker and all other
     lines.
   - An existing **anchored** `/{basename}/` line (or the marker) short-circuits
     to a no-op (idempotent).

   Implementation sketch: read lines; if any line trimmed == `/rig/` (or the
   marker) → no-op; else if any line trimmed == `rig/` → replace that line with
   `/rig/` and write back; else fall through to the append path. Keep the
   existing tracked-by-parent guard before writing.

   The tracked-by-parent `git ls-files` check continues to use the bare
   `basename` (git pathspec), unchanged.

3. **Update log messages** to reference the anchored entry (they already embed
   `{entry}`, so they pick up the leading slash automatically).

**Tests (update + add):**
- `ensure_rig_repo_appends_marked_entry_preserving_existing` → assert
  `l.trim() == "/rig/"`.
- `ensure_rig_repo_is_idempotent` → assert the anchored entry appears once;
  run twice.
- `ensure_rig_repo_uses_rig_basename_for_named_rigs` → assert `/dev-rig/`.
- **New** `ensure_rig_repo_migrates_unanchored_entry`: pre-write `.gitignore`
  containing a bare `rig/` line (plus a sentinel line), run
  `ensure_rig_repo`, assert the line is now `/rig/`, the sentinel survives, and
  no duplicate was appended.
- **New** `ensure_rig_repo_leaves_anchored_entry` (idempotence after
  migration): pre-write `/rig/`, assert content unchanged after a run.

### Phase 3: Verification and version bump

- `cargo test` — all suites green.
- `cargo clippy` — no new warnings.
- Bump the binary version (patch — `0.41.0` → `0.41.1`) and reinstall per
  `AGENTS.md` (`cargo install --path .`). Do **not** run the rig here.

## Notes

- **Why force-migrate rather than accept both forms:** leaving bare `rig/` lines
  in place means the over-match silently persists for every existing rig and
  depends on a manual edit nobody makes. Rewriting in place is a one-time,
  content-preserving fix that converges every install on the correct anchored
  form.
- The migration only ever *adds a leading slash* to a line Knot itself wrote.
  It must not touch any other line — user-authored `.gitignore` entries are
  preserved verbatim.
- If a rig directory legitimately contains a nested directory also named after
  the rig, only the root-level one is now ignored; that is the intended,
  corrected behaviour (`tie-offs/rig/` and similar stay versioned).
