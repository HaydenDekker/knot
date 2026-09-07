# Plan 083: Consolidated Service Log + Change-Driven State Writes

## Related Plans

Supersedes the startup-clear mechanism introduced by
[072 Startup Log Clear](../072-startup-log-clear/startup-log-clear-plan.md)
(the logs it cleared stop existing as files; the durable history instead
lives in `knot-service.log`, which 072 already identified as the only log
that survives a restart). Coordinates with
[082 System Event Subscriptions](../082-system-event-subscriptions/system-event-subscriptions-plan.md)
— its "human-readable body line per event" is exactly the `[EVENT]` line
this plan standardises; the shared render function is the seam. Builds on
[064 Local Timestamps](../064-local-timestamps/) (line timestamp format)
and [065 Persistent Event Queue](../065-persistent-event-queue/)
(`strand_queue` entries in state).

## Problem

A user who wants to watch a rig today has three artifacts to read:

1. **`tie-offs/<rig>/knot-service.log`** — the launcher's raw
   stderr/stdout capture (appended across runs). Carries service
   lifecycle lines (`[CONFIG]`, `[STRAND]`, `[WATCH]`, warnings) and
   nothing else.
2. **`tie-offs/<rig>/.rig-log`** — JSONL, 2 `RigLogEvent` variants
   (`TimeoutExceeded`, `QueueIdle`), cleared at startup.
3. **`tie-offs/<rig>/<loom-id>/.loom-log`** (one per loom) — JSONL, 18
   `LoomEvent` variants, cleared at startup.

The operational story of a run — which knot picked up which strand, what
failed, what dispatched to whom — lives only in (2) and (3), so "tail one
file" does not work: the user must know which loom dirs exist and read
them all. Meanwhile (1), the file that *survives* restarts, holds the
least run-specific content.

State has the opposite problem. `state.json` is **rewritten every 5
seconds unconditionally** (`server.rs::start_state_writer`): a full
pretty-printed `RigState` (all looms, knots, profiles, queue) is
serialised and atomically renamed even when nothing changed — `updated_at`
churns every 5 s, `mtime` churns every 5 s, and a watcher polling the file
cannot use "file changed" as a "state changed" signal. There is no record
anywhere of *what* changed.

## Target

### 1. `knot-service.log` is the one log

Every line a user needs is in `tie-offs/<rig>/knot-service.log`:

- service lifecycle lines (as today: `[CONFIG]`, `[STRAND]`, `[WATCH]`,
  `[LOOM]`, `[KNOT]`, warnings, panics, raw agent output), **plus**
- one `[EVENT]` line per rig-log / loom-log event (all 20 variants),
  **plus**
- `[STATE]` lines recording state changes.

The per-loom `.loom-log` files and the `.rig-log` file are **retired**.
The JSONL event *types* (`LoomEvent`, `RigLogEvent`) remain the canonical
event model; they stop being persisted to files and are rendered to
`[EVENT]` lines instead.

Ownership of the file is unchanged: the service writes everything to
stderr; the `knot-start` launcher keeps capturing stderr into
`knot-service.log` (append across runs; rotation to `.1` stays a
skill-owned operation). No new file writer, no double-capture: the
`[EVENT]`/`[STATE]` lines are part of the same stderr stream the launcher
already records, so panics, live agent output, and raw lines keep landing
in the same file (see Notes, "Service owns the stream, not the file").

Consequence: the consolidated log is **durable across runs** (the
launcher appends), while the in-process activity queries remain
current-run scoped — the exact split 072 already drew between the
service log ("the only Knot log that survives a restart") and the
cleared operational logs.

### 2. Every log event is echoed

At every site that today appends a `LoomEvent` / `RigLogEvent`, the event
is also rendered to one `[EVENT]` line (same fields, human-readable,
`k=v` pairs, single line, stable field order — greppable and
future-parseable). Examples:

```
[2026-09-07T14:03:22+01:00] [KNOT][EVENT] LoomStarted loom=prds-loom
[2026-09-07T14:03:22+01:00] [KNOT][EVENT] KnotRegistered loom=prds-loom knot=review
[2026-09-07T14:03:25+01:00] [KNOT][EVENT] KnotProcessing loom=prds-loom knot=review strand=strands/prd.md
[2026-09-07T14:05:01+01:00] [KNOT][EVENT] KnotCompleted loom=prds-loom knot=review strand=strands/prd.md tie-off=tie-offs/rig/prds-loom/prd.md
[2026-09-07T14:05:01+01:00] [KNOT][EVENT] EventsDispatched loom=prds-loom knot=review dispatch=PlanCreated→docs-loom/write: tie-offs/rig/docs-loom/PlanCreated/event-2026-09-07T14-05-01Z.md
[2026-09-07T14:05:02+01:00] [KNOT][EVENT] QueueIdle
[2026-09-07T14:05:02+01:00] [KNOT][EVENT] TimeoutExceeded loom=prds-loom knot=review strand=strands/big.md error=session exceeded 300s
```

### 3. State changes are events, in the same log

When `state.json` is (re)written, the **delta** since the previous write
is logged as one `[STATE]` line per changed aspect — never the whole
model:

```
[2026-09-07T14:03:22+01:00] [KNOT][STATE] initial snapshot looms=2 knots=5 profiles=3 queue=0
[2026-09-07T14:03:30+01:00] [KNOT][STATE] change knot prds-loom/review: status idle→processing strand=strands/prd.md
[2026-09-07T14:03:30+01:00] [KNOT][STATE] change queue+ strands/prd.md (created, prds-loom/review)
[2026-09-07T14:05:06+01:00] [KNOT][STATE] change knot prds-loom/review: status processing→completed tie-off=tie-offs/rig/prds-loom/prd.md
[2026-09-07T14:05:06+01:00] [KNOT][STATE] change queue- strands/prd.md
[2026-09-07T14:05:06+01:00] [KNOT][STATE] change profile fast: model gpt-4o→o3
[2026-09-07T14:09:12+01:00] [KNOT][STATE] change loom+ docs-loom
[2026-09-07T14:09:12+01:00] [KNOT][STATE] change knot+ docs-loom/write
```

(`+`/`-` = added/removed. A quiet rig produces **zero** `[STATE]` lines
between changes — silence means no change.)

### 4. `state.json` is written only on change

The 5-second poll stays (building a snapshot is cheap once knot-status
derivation is in-memory — see Design), but the **write** is conditional:

- build the candidate snapshot (with `updated_at` normalised out of the
  comparison),
- compare it (structural `PartialEq`) against the snapshot last written
  by this process,
- if identical **and** `state.json` exists on disk → skip: no write, no
  log line, no `mtime` churn,
- if different (or the file is missing externally) → compute the delta,
  stamp `updated_at = now`, write atomically (existing
  `FileSystemStateWriter`, unchanged), emit the `[STATE]` lines.

The first write of every process is always a write (baseline, logged as
`initial snapshot looms=N knots=N profiles=N queue=N` — counts only, not
the model). `updated_at` therefore now means *last time state actually
changed* (previously *last time we rewrote whatever state was there*);
`knot step` processes get the same treatment (baseline write at start,
delta write after the stepped event).

### 5. What "the log" is for code, after consolidation

Today the JSONL files have three code consumers:

| Consumer | Today reads | After this plan |
|---|---|---|
| `WriteState::derive_knot_state` (knot status in state) | per-knot `read_all` of the loom-log **file, every 5 s, per knot** | the in-memory current-run activity store (same port call, no I/O, no parse, no WARN-on-bad-line noise) |
| `get_activity` / `get_knot_status` queries | loom-log files | the same in-memory store (current-run scoped, as 072 already made the logs) |
| external watchers (082's rig-log rationale, skills) | JSONL files | grep the consolidated log; durable history is now *better* than before (survives restarts instead of being cleared) |

Knot-status continuity check (verified against today's behaviour): a
fresh process re-registers all knots at discovery (`KnotRegistered` is
the latest event ⇒ status `idle`). Today that re-registration is appended
to the same (service: cleared / step: accumulated) file, so *today*
post-restart and post-step statuses are also re-based to `idle`.
In-memory scoping therefore changes no status semantics — it only removes
the file round-trip.

## Non-Goals

- No new file writer in the service — the service keeps printing to
  stderr; the launcher keeps capturing (ownership model unchanged).
- No log rotation/size limiting inside the service (skill-owned, as
  today: `knot-service.log.1`).
- No change to the `state.json` **schema** (`RigState` JSON is
  byte-compatible; only write frequency and `updated_at` semantics
  change) and no change to atomic-write mechanics.
- No event-driven (push) state writes — the 5 s poll remains; only the
  write becomes conditional.
- No change to tie-off format, event dispatch, queue behaviour, or
  agent-facing prompt context.
- No 082 subscription mechanics (that plan lands the *dispatch* side;
  this plan standardises the *render* side it reuses).
- No JSONL parsing of `knot-service.log` by Knot itself (consumption is
  grep/tail-based, human/skill-facing).

## Design

### Log line format

Extend `src/adapters/logging.rs` with two renderers + a state renderer.
All new lines use the existing `[ts] [KNOT][TAG]` prefix (064 local-time
ISO 8601), one event per line, `k=v` fields, paths as stored
(project-relative where the event already carries them):

```rust
pub fn log_loom_event_line(event: &LoomEvent)   // [KNOT][EVENT] <Variant> k=v …
pub fn log_rig_event_line(event: &RigLogEvent)  // [KNOT][EVENT] <Variant> k=v …
pub fn log_state_change_lines(change: &StateChange, snapshot: Option<&RigState>)
    // [KNOT][STATE] initial snapshot looms=… (prev=None)
    // [KNOT][STATE] change <aspect> … (prev=Some)
```

Field names per variant mirror the struct fields
(`loom=`, `knot=`, `strand=`, `tie-off=`, `error=`, `attempt=`,
`session=`, `silent=`, `window=`, `blocked-call=`, `reason=`,
`tokens-before=`, `directory=`, `file=`, `message=`, `expected=`,
`dispatch=` repeated per dispatch). A trailing free-text `— detail`
segment is **not** used for `[EVENT]` lines (machine-greppability wins;
the existing `[LOOM]`/`[KNOT]` lifecycle helpers keep their current
prose style).

Renderers are pure functions over the domain types (testable per
variant); the `eprintln!` happens once at the end (one physical line per
event, so the launcher's capture is line-atomic with the JSONL lines it
replaces).

### Domain (`src/domain/`)

**`events.rs`** — `LoomEvent` / `RigLogEvent` unchanged (serde derives
kept; they are harmless and 082 may reuse them).

**New `state_change.rs`** (or in `entities.rs` next to `RigState`):

```rust
pub struct StateChange {
    pub looms_added: Vec<String>,
    pub looms_removed: Vec<String>,
    pub knots_added: Vec<(String, String)>,          // (loom, knot)
    pub knots_removed: Vec<(String, String)>,
    pub knot_updates: Vec<KnotUpdate>,               // see below
    pub profiles_added: Vec<String>,
    pub profiles_removed: Vec<String>,
    pub profile_updates: Vec<ProfileUpdate>,
    pub queue_added: Vec<RigStateStrandQueueEntry>,
    pub queue_removed: Vec<RigStateStrandQueueEntry>,
}

pub struct KnotUpdate {
    pub loom: String,
    pub knot: String,
    /// One entry per changed field: status (old→new), last-strand,
    /// last-tie-off, last-error, last-event-at.
    pub field_changes: Vec<(KnotField, Option<String>, Option<String>)>,
}

pub struct ProfileUpdate {
    pub name: String,
    pub field_changes: Vec<(ProfileField, Option<String>, Option<String>)>,
}
```

**`diff_state(prev: Option<&RigState>, new: &RigState) -> StateChange`** —
pure function:

- looms by `id`; knots by `id` within a loom; profiles by `name`.
- knot fields compared individually (`status`, `last_strand_path`,
  `last_tie_off_path`, `last_error`, `last_event_at`); a status change
  renders `idle→processing` (from/to), other fields render new value
  (from shown only when non-trivial: e.g. error cleared).
- profile fields compared individually (`model-ref`, `provider`, `model`,
  `thinking-level`, `timeout`).
- queue entries keyed by the queue's own `dedup_key`
  (`(strand_path, loom_id, knot_id, kind)` — `domain/pending_event.rs`),
  so a re-queued same-strand event of a different kind is add+remove,
  not a no-op.
- `updated_at` and `rig_path` are **excluded** from the diff
  (`updated_at` is the write stamp; `rig_path` is process-constant). The
  equality check used for skip-decision normalises `updated_at`
  identically before comparing.

### Application (`src/application/`)

**In-memory activity store** (new, `src/application/activity.rs`):

```rust
pub struct RunActivity {
    loom_events: RwLock<HashMap<String /*loom*/, Vec<LoomEvent>>>,  // current run, newest last
    rig_events:  RwLock<Vec<RigLogEvent>>,
}
```

- Bounded ring per loom (cap 10 000 events, drop oldest — a run produces
  hundreds; the cap is insurance, not expected).
- `append_loom(&LoomEvent)`, `append_rig(&RigLogEvent)`,
  `loom_events(&LoomId)`, `rig_events()` — the data the port methods
  below delegate to.

**Ports (`ports.rs`)** — `LoomLogPort` / `RigLogPort` keep their names
and their `open`/`append`/`read_all` contracts (so `WriteState`,
`get_activity`, `get_knot_status`, and the config/loom use cases need no
signature changes); **`clear` / `clear_all` are removed** (and the
`StartupOptions::clear_logs` gate in `run_startup` — see Server). The
in-memory store *is* the current-run scope: a fresh process starts
empty, which is exactly what the startup clear used to guarantee, minus
the I/O.

**New production adapters** (replace `FileSystemRigLog` /
`FileSystemLoomLog` + their `Shared*` wrappers):

- `InMemoryLoomLog` (impl `LoomLogPort`): `append` = store insert **and**
  `logging::log_loom_event_line(&event)`; `read_all` = store read;
  `open` = no-op (the loom dir is created where it matters today:
  tie-offs/dispatch dirs).
- `InMemoryRigLog` (impl `RigLogPort`): same shape.
- One `Arc<RunActivity>` per process, built in `build_app_context`
  (replacing the two `FileSystem*` constructions at `server.rs:~267`).

**`WriteState` (`usecases/write_state.rs`)** — change detection:

```rust
pub struct WriteState {
    … existing fields …,
    last_written: Option<RigState>,   // updated_at normalised (empty)
}
```

- `execute()` becomes: `let candidate = build_state_with_updated_at(EMPTY);`
  → `if last_written == Some(candidate) && state.json exists on disk {
  return Ok(()) }` → `let change = diff_state(last_written.as_ref(),
  &candidate);` → `candidate.updated_at = now; state_writer.write_state(…);
  last_written = Some(candidate); log_state_change_lines(…);`
- `build_state()` is otherwise unchanged — `derive_knot_state` keeps
  deriving from `log_port.read_all` (now the in-memory store: no file
  I/O, no parse, no `WARN:` noise per cycle, which removes 072's
  console-pollution failure mode permanently).
- Force-write on missing file keeps `state.json` self-healing against
  external deletion (corruption: out of scope — readers see the old/absent
  file and report it).
- `start_state_writer` keeps the 5 s tick and the immediate first write
  (now logged as `initial snapshot …`). A `KNOT_STATE_WRITE_MS` env
  override (mirroring `KNOT_TEST_CHECK_MS`) is added so the integration
  harness can drive write timing deterministically.

### Server / startup (`src/server.rs`)

- Remove the `clear_logs` gate (`run_startup` ~L868), the
  `StartupOptions.clear_logs` field, and the `Service`/`step`
  constructor distinction it created (both just become the one option
  set). No file is left to clear.
- The pipeline's `QueueIdle` write (`server.rs` ~L517) and the shutdown
  `LoomStopped` writes (`~L1127`, `~L1256`) keep calling the same port
  methods — they now land in the store + the consolidated log instead of
  a file; drop the redundant `eprintln!("[pipeline] QueueIdle written…")`
  (the `[EVENT]` line replaces it).
- `knot step` (`step_knot`): unchanged flow; its `start_state_writer`
  gives the baseline + post-step delta writes and their `[STATE]` lines,
  and the skill's `tee -a knot-service.log` puts the step's `[EVENT]`
  lines in the same consolidated log as the service's.

### Files removed / retired

- `src/adapters/outbound/rig_log.rs` + `loom_log.rs` (JSONL writers,
  readers, `Shared*` wrappers, `clear`/`clear_all`) — replaced by
  `src/application/activity.rs` + the two in-memory adapters.
- `derive_loom_log_path` / any loom-log-path helpers in
  `domain/knot_file.rs` (verify remaining uses first; the loom *dir*
  derivation is still needed for tie-offs/dispatch and stays).
- On-disk artifacts: `.rig-log`, `*/.loom-log` stop being created.
  **Existing files in the wild are left as-is** (orphaned, inert — no
  code reads them after this lands; the skills stop referencing them).
  No deletion-by-Knot (never delete user files on upgrade).

## Phases

### Phase 0: Failing tests

Unit (domain):

- `diff_state`: per-aspect add/remove/update (loom, knot, knot field,
  profile field, queue add/remove via `dedup_key`); `updated_at` /
  `rig_path` excluded; `prev=None` ⇒ everything "added" (rendered as the
  initial snapshot instead); empty `StateChange` when only
  `updated_at` differs.
- Renderers: every `LoomEvent` (18) and `RigLogEvent` (2) variant →
  exact expected line (field order pinned); `StateChange` → exact lines
  (status `old→new`, `+`/`-`, queue add/remove, profile field change);
  initial-snapshot line shape.
- `RunActivity`: append/read per loom, rig list, ring cap.

Unit (application):

- `InMemoryLoomLog` / `InMemoryRigLog` implement the ports (round-trip
  `append`→`read_all`); `open` no-op; no file created in a temp dir.
- `WriteState::execute`: first call writes (baseline); second call with
  unchanged inputs writes nothing (mock `StateWriterPort` sees 1 write
  total, no `[STATE]` line beyond the initial snapshot); after a store
  mutation (new `KnotProcessing` event / profile edit / queue push) the
  next call writes and the delta contains exactly the changed fields;
  deleted `state.json` forces a rewrite with an unchanged delta-free
  snapshot (the writer is called again, line is `state file missing —
  rewritten` or the initial-snapshot shape — decision pinned in the test).

Integration (`tests/`, mock-CLI harness — no live rig runs per AGENTS.md):

- A full run (strand → processing → tie-off) produces the expected
  `[EVENT]` lines in the captured stderr (assert order + fields), and
  **no** `.rig-log` / `.loom-log` files exist under the runtime root
  afterwards.
- `state.json`: written at start (baseline), rewritten only on real
  changes — assert `mtime`/content stability across an idle window and
  that the `[STATE]` line count equals the number of change ticks;
  `updated_at` equals the last change, not "now".
- Burst: two strands queued back-to-back ⇒ queue+ lines for both, then
  two queue- lines; the delta lines never contain an unchanged aspect
  (assert absence, e.g. no `profile` line during a pure queue churn).
- `knot step` in a step session: baseline + delta lines captured via
  `tee`; post-step state shows the stepped knot's new status.

### Phase 1: Domain

`StateChange` + `diff_state` + the render functions (pure, no I/O);
renderers land in `src/adapters/logging.rs` (they are logging concerns)
with the domain types in `domain/`.

### Phase 2: In-memory activity + adapter swap

`RunActivity`, `InMemoryLoomLog`, `InMemoryRigLog`; port trait updates
(remove `clear`/`clear_all`); mock updates in `ports.rs` and
`write_state.rs` tests; `build_app_context` swap; `run_startup`
simplification (drop `clear_logs` + gate); delete the two JSONL adapters
and `derive_loom_log_path` (after use-check). All `append` call sites
untouched (same port calls) — this is what keeps the blast radius small.

### Phase 3: Change-driven state writes

`WriteState` comparison + `diff_state` + `[STATE]` logging;
`KNOT_STATE_WRITE_MS`; the `QueueIdle`/`LoomStopped` eprintln
deduplication.

### Phase 4: Integration + regression

Phase-0 integration tests green; full `cargo test`, `cargo clippy`;
existing suites (056/058/059/070 event routing, 065 persistent queue,
`tests/filesystem_interface.rs`, `tests/pipeline.rs`) green unchanged —
they assert on tie-offs/queue/state, not on the retired log files.
Known exceptions to convert from file assertions to captured-stderr / in-
memory assertions: `tests/adapters.rs`, `tests/helpers.rs`,
`tests/late_removal.rs`, `tests/multi_loom.rs`, `tests/queue_identity.rs`,
`tests/rig_cli.rs`, `tests/step.rs` (19 `.rig-log`/`.loom-log`
references today).

### Phase 5: Skills, docs, version

- **knot-inspect**: activity source → `knot-service.log` (grep
  examples: `grep '\[EVENT\] .*loom=prds-loom' tie-offs/rig/knot-service.log`);
  `updated_at` semantics ("last actual change"); layout section drops
  `.loom-log` / `.rig-log`.
- **knot-analyst**: replace `.rig-log` / `.loom-log` read recipes with
  service-log greps (`TimeoutExceeded`, `QueueIdle`, `KnotFailed` as
  `[EVENT]` lines); the "rig-log does not exist" fallback becomes
  "no `[EVENT]` lines — no serious events recorded".
- **knot-manage**: `EventsDispatched` review greps the service log.
- **knot-start**: runtime-file table (drop the two log rows; the service
  log row now says "service lines + `[EVENT]` lines + `[STATE]` lines");
  run-boundary note (each start's first lines are `KnotRegistered` /
  `LoomStarted` `[EVENT]` lines after the last startup banner);
  verification snippets updated.
- **knot-create / knot-init**: runtime-tree diagrams drop `.loom-log` /
  `.rig-log`; knot-glossary entries (loom-log, rig-log, runtime root,
  directory layout) rewritten for the consolidated log.
- **AGENTS.md**: the note "the only Knot log that survives a restart (the
  rig-log and loom-logs are cleared at startup)" becomes "the
  consolidated log — service lines, `[EVENT]` lines, `[STATE]` lines".
- **knot-update**: changelog entry for the next minor version
  (0.41.0 if this lands before 082, else 0.42.0 — coordinate): breaking
  — `.rig-log` / `.loom-log` files no longer written (existing files
  inert); `state.json` written only on change (`updated_at` = last
  actual change); new `[EVENT]` / `[STATE]` line vocabulary; viewing
  recipes move to `knot-service.log`. No rig-document migration
  (loom/knot/profile files unchanged).
- **Release notes + README/docs** (troubleshooting "view logs",
  getting-started observability paragraph).
- Version bump; `cargo install --path .` after project-plan-completion;
  deploy skills per AGENTS.md (copy + diff verify).

## Test Strategy

- **Unit**: `diff_state` (pure, exhaustive per aspect), renderers
  (per-variant line pinning), `RunActivity` (ring + per-loom reads),
  in-memory port adapters, `WriteState` skip/force-write logic against
  the existing mock `StateWriterPort`.
- **Integration**: mock-CLI harness — event-line capture (order +
  fields), no-retired-files assertion, state write frequency (content +
  mtime stability across idle window), delta content (changed aspects
  present, unchanged aspects *absent*), `knot step` baseline+delta.
- **Regression**: all existing suites green; suites asserting on the
  retired JSONL files are the only ones rewritten (audited in Phase 4).
- No live rig runs in this repository (AGENTS.md) — verification is
  `cargo test` / `cargo clippy` + the mock-CLI harness.

## Notes — design rationale

- **Service owns the stream, not the file.** `knot-service.log` is the
  launcher's capture (AGENTS.md, knot-start skill: "the only Knot log
  that survives a restart"). Having the service open the file too would
  double-write under the launcher's `>> … 2>&1` (or force the skill to
  discard stderr, losing panics and live agent output). Echoing into the
  existing stderr stream gets consolidation with zero ownership change
  and keeps the file's raw-capture character (structured lines among raw
  lines, as today).
- **Files retired, not kept in shadow.** Keeping the JSONL files as
  machine-readable duplicates would leave two sources of truth and
  re-create the "which file do I read" problem this plan solves; every
  code consumer of the files is re-routed to the in-memory store (which
  also deletes the per-5-s per-knot file re-read + JSON parse and the
  072 WARN-noise failure mode). The skills and any external watcher move
  to grep — and gain durability for free: history that 072 *cleared* at
  every startup now survives in the appended service log (rotation
  remains the escape hatch).
- **In-memory activity is current-run scoped — and that matches today.**
  Statuses are re-based to `idle` by discovery's `KnotRegistered` in a
  fresh process regardless of file persistence (verified: step mode
  accumulates the file but re-registration still appends the latest
  event), so dropping the files changes no status semantics. Durable
  history is the service log's job; 072 already declared the logs'
  cross-run content "residue that nothing consumes" — the tie-offs
  (git-versioned) remain the durable *audit* record.
- **Compare-then-write, not event-driven writes.** The 5 s poll is cheap
  after Phase 2 (no I/O in `build_state` beyond profile/registry reads);
  making writes conditional delivers "write only on change" without a
  push plumbing from every store mutation into the writer. Event-driven
  writes stay a future optimisation (the `[STATE]` lines lag events by
  at most one tick — acceptable for a 5 s observability cadence).
- **`updated_at` exclusion is the whole ballgame.** Without normalising
  the timestamp out of the comparison, every build differs and the skip
  never fires. `rig_path` is process-constant and excluded for the same
  reason; if it ever varies (multi-rig one process — out of scope), the
  diff will simply report it.
- **Queue diff uses the queue's own `dedup_key`.** `(strand, loom, knot,
  kind)` is what `push_or_replace` treats as "the same pending event",
  so state-delta and queue-identity cannot disagree.
- **Deltas are per-aspect lines, not one snapshot line.** One line per
  changed aspect keeps `tail`/`grep` useful (a user greps
  `change knot prds-loom/review:` and sees that knot's history), and
  multi-change snapshots cost only a few lines. Volume is bounded by
  actual work: a quiet rig is silent, a burst costs one line per queued
  strand.
- **Force-write only on missing file.** Corruption is out of scope
  (detecting "different on disk" means reading + parsing the file every
  tick — the very churn we are removing). Missing-file detection is one
  `exists()` call and covers the realistic accident (manual cleanup).
- **Backwards compatibility.** `state.json` schema is unchanged;
  `LoomEvent`/`RigLogEvent` types are unchanged. The only breaking
  surface is the retired files — documented in the knot-update changelog,
  and acceptable because (a) nothing in-repo consumes them after this
  plan, (b) 082 (the would-be external-watcher consumer) is not started
  and will target the consolidated log, and (c) skills are deployed in
  the same release.

## Implementation Status: ✅ Complete (2026-09-07) — released in v0.41.0
