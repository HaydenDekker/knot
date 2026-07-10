# Plan: Strand Event URI

## Related PRD

No PRD exists for agent-to-agent event routing. The feature was implemented in Plan #45 without a corresponding PRD. This plan is a refactor of the knot configuration model used by that feature to eliminate dual input directions on a single knot.

## Problem

Consumer knots currently declare event subscriptions using `listens-for` — a YAML array of `Intent` objects (`target-knot`, `event-id`, `event-description`) — alongside a normal `strand-dir` filesystem path. This gives a knot **two input directions** (file system + event dispatch), breaking the "one strand, one direction" principle and allowing implicit fan-in. The `listens-for` array is structurally different from `strand-dir` even though both define where a knot gets its input from.

## Target

Knots have exactly **one** input direction, expressed in `strand-dir`:

- **Normal knots**: `strand-dir: "project/prds"` — a filesystem path (unchanged)
- **Event consumer knots**: `strand-dir: "event:completion-validator:NonConformance"` — a URI scheme that encodes the producer knot and event ID

An optional `event-description` field on the consumer knot provides the semantic description injected into the producer's prompt. When absent, a generic prompt is injected.

The `listens-for` array, `Intent` struct, and `matches_intent()` function are removed. `StrandSource` enum replaces them as the single input-direction primitive. `build_listener_context()` and event dispatch logic scan `strand_source` instead of `listens_for`.

### Improved Context Injection Format

The injected context block is improved:
- Has a **markdown heading** so it's visually distinct from knot instructions
- Producer is told to **always emit an event block** — either a real event or `event: None`
- Event block includes a **`description`** field (required for non-None events)
- Producer does **not** see consumer knot names — only the events it may emit

Target injected format:

```
## Agent Events

Other knots are listening for events you may emit. If an event occurs
during your work, include an explicit event block in your tie-off using
the format shown.

Events you may emit:
- `NonConformance` — Emitted when validation of a CI job fails one or
  more BDD scenarios. Contains the plan, CI job, failed scenarios with
  evidence, and a gap summary.

If an event occurred, emit in your tie-off:
```
event: NonConformance
description: <short summary of what happened>
<additional fields as relevant>
```

If no events occurred, emit:
```
event: None
```
```

### Event Contract

- `event:` is **always present** in the structured tie-off block
- When `event:` has a real ID, `description:` is **required**
- When `event: None`, no other fields are needed
- `target-knot` is **not emitted by the producer** — the system knows which knot produced the tie-off; it was redundant
- The tie-off event parser must recognise `event: None` as a valid signal (no dispatch occurs)

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `domain::knot_file::tests::*_listens_for*` (7 tests) | `listens-for` parsing, serialisation, defaults | ✅ Green |
| `domain::events::tests::*intent*` (13 tests) | `Intent` struct, `matches_intent()`, `build_listener_context()` | ✅ Green |
| `domain::entities::tests::*event_metadata*` (8 tests) | `EventMetadata` struct and serialisation | ✅ Green |
| `domain::entities::tests::*agent_event*` (3 tests) | `AgentEvent` struct and serialisation | ✅ Green |
| `adapters::outbound::event_dispatcher::tests::*` (8 tests) | Event file creation, fan-out, content structure | ✅ Green |
| `application::usecases::process_strand::event_dispatch_tests::*` (4 tests) | Full dispatch flow, context injection, fan-out | ✅ Green |
| `application::usecases::process_strand::event_metadata_tests::*` (10 tests) | Event metadata extraction from event files | ✅ Green |
| `application::usecases::process_strand::tieoff_event_metadata_tests::*` (3 tests) | Tie-off append with event metadata | ✅ Green |
| `adapters::pi_stdio::tests::runner_passes_event_metadata` (1 test) | Event metadata through runner | ✅ Green |
| `domain::entities::tests::*knot*` (knot construction tests) | Knot entity construction with `listens_for` | ✅ Green |
| `domain::knot_file::tests::knot_file_with_listens_for_serialization_roundtrip` | `KnotFile` serialisation with `listens_for` | ✅ Green |
| `domain::knot_file::tests::knot_file_serialization_with_empty_listens_for` | Empty `listens_for` serialisation | ✅ Green |

**Total tests affected: ~57** (all `listens_for` / `Intent` / `build_listener_context` / event dispatch / event metadata tests)

## Test Gaps

- No tests for `StrandSource` enum (does not exist yet)
- No tests for `event:` URI parsing
- No tests for `event-description` field parsing
- No tests for `build_listener_context()` working from `StrandSource::EventUri`
- No tests for event dispatch matching consumers by `StrandSource`
- No tests for unified watcher (event directory creation + watch in one function)
- No test verifying `listens-for` becomes an unknown property warning after removal

## Phases

### Phase 0: Domain — StrandSource enum, Intent removal from Knot/KnotFile

Introduce `StrandSource` enum in domain, add `event_description` to `KnotFile`/`Knot`, remove `listens_for`.

- [ ] Define `StrandSource` enum in `src/domain/value_objects.rs` (or new file):
  - [ ] `Filesystem(PathBuf)` — normal strand directory
  - [ ] `EventUri { producer_knot: String, event_id: String }` — event subscription
- [ ] Add `from_str(s: &str) -> Result<StrandSource, KnotFileError>` — parses plain paths and `event:` URIs
- [ ] Add `is_event(&self) -> bool` helper
- [ ] Add `StrandSourceError` variant to `KnotFileError` for malformed URIs
- [ ] Replace `listens_for: Vec<Intent>` with `strand_source: StrandSource` on `KnotFile`
- [ ] Add `event_description: Option<String>` to `KnotFile` (new optional field, serialised as `event-description`)
- [ ] Replace `listens_for: Vec<Intent>` with `strand_source: StrandSource` on `Knot`
- [ ] Add `event_description: Option<String>` to `Knot`
- [ ] Remove `default_listens_for()` and `Intent` import from `entities.rs`
- [ ] Update all knot construction tests to use `StrandSource::Filesystem(path)`
- [ ] **Tests**: `StrandSource` construction, `from_str` for plain path, `from_str` for event URI, `from_str` for malformed URI, `is_event()` helper, `Knot` construction with both variants, serialisation round-trips

### Phase 1: Knot File Parsing — event: URI scheme, event-description, strand_dir removal

Update `KnotFile::parse()` to handle `strand-dir` as either a plain path or `event:` URI.
Remove the redundant `strand_dir: PathBuf` field — `strand_source: StrandSource` is the single source of truth.

- [x] In `RawFrontmatter`, add `event_description: Option<String>` (YAML key: `event-description`)
- [x] In `parse()`, parse `strand-dir` string through `StrandSource::from_str()` — this is now the **only** place the raw string is consumed
- [x] On malformed `event:` URI (missing parts, wrong scheme), return `KnotFileError::StrandSourceError`
- [x] Propagate `event_description` from raw to `KnotFile`
- [x] **Remove `strand_dir: PathBuf` from `KnotFile`** — `strand_source` is the single representation
- [x] **Remove `strand_dir: PathBuf` from `Knot`** — `strand_source` is the single representation
- [x] Add `StrandSource::path() -> Option<&Path>` helper for consumers that need the filesystem path (returns `Some` for `Filesystem`, `None` for `EventUri`)
- [x] Update `knot_from_file()` in `loom_repository.rs` — remove `strand_dir` field, pass `strand_source` through
- [x] Update `resolve_path()` callers in `loom_repository.rs` — resolve path from `strand_source.path()` instead of `strand_dir`
- [x] Update all other `strand_dir` consumers (watchers, event_source, process_strand, tests) to use `strand_source`
- [x] **Tests**: Parse valid event URI, parse event URI with event-description, parse plain path (unchanged), parse malformed event URI returns error, parse event URI with missing producer returns error, `listens-for` becomes unknown property warning, `path()` helper returns correct path for Filesystem, `path()` returns None for EventUri, `KnotFile` serialisation no longer contains `strand_dir`, `Knot` serialisation no longer contains `strand_dir`

### Phase 2: Context Injection — build_listener_context from StrandSource (improved format)

Refactor `build_listener_context()` to scan `strand_source` instead of `listens_for`, and produce the improved format.

- [ ] Change signature to `build_listener_context(knot: &Knot, all_knots: &[Knot]) -> String`
- [ ] Scan all knots' `strand_source` for `EventUri { producer_knot == this_knot.id.0 }`
- [ ] Group by `event_id`, deduplicate (same as before)
- [ ] Use `event_description` from the consumer knot (or generic message if `None`)
- [ ] **New format**: markdown heading (`## Agent Events`), no consumer knot names visible to producer, instruct producer to always emit an event block (real event or `event: None`), `description` field required for non-None events
- [ ] Remove `Intent` dependency from the function
- [ ] **Tests**: Output has heading, output contains event descriptions, output does NOT contain consumer knot names, output instructs `event: None` for no-events, output requires description field, single consumer triggers context, no consumers returns empty, multiple consumers same event deduplicates, multiple different events from same producer, generic message when event-description is None

### Phase 3: Event Dispatch — Match consumers by strand_source, handle event: None

Refactor the event dispatch matching flow and update the tie-off event parser for the new format.

- [ ] **Tie-off parser changes** (`tieoff_parser::extract_agent_events`):
  - [ ] `target-knot` is no longer emitted by the producer — the caller (ProcessStrand) fills it from the knot producing the tie-off
  - [ ] `event: None` is a valid signal — when parsed, no `AgentEvent` is produced (skip dispatch)
  - [ ] `description` is a new recognised payload field (no special handling, just passes through)
  - [ ] `AgentEvent` struct: remove `target_knot` field (derived at dispatch time), keep `event_id` and `payload`
- [ ] **Dispatch matching**: scan all knots for `strand_source: EventUri { producer_knot, event_id }` where `event_id` matches the parsed event and `producer_knot` matches the producing knot's ID (known from context, not from the event)
- [ ] Remove `matches_intent(event, intent)` function from `events.rs`
- [ ] Remove `Intent` struct from `events.rs` (no longer needed)
- [ ] **Tests**: Full dispatch flow with EventUri consumers, `event: None` produces no dispatch, fan-out two looms with EventUri, no consumers no dispatch, event ID mismatch no dispatch, description field passes through to event payload, target-knot derived from producing knot context

### Phase 4: Watchers — Unified strand directory and event directory handling

Merge `ensure_strand_dir_and_watch()` and `ensure_event_watches()` into a single function that handles both `StrandSource` variants.

- [ ] Rename/replace `ensure_strand_dir_and_watch()` with a single function that:
  - [ ] For `StrandSource::Filesystem(path)` — create if missing, watch (same as current)
  - [ ] For `StrandSource::EventUri { event_id }` — derive `rig/tie-offs/{loom-id}/{event-id}/`, create if missing, watch
- [ ] Remove `ensure_event_watches()` entirely
- [ ] Update callers in `discover.rs`, `register.rs`, `config_event_handler.rs` to call the unified function once per knot (instead of calling both `ensure_strand_dir_and_watch` and `ensure_event_watches`)
- [ ] **Tests**: Filesystem source creates and watches directory, EventUri source derives and creates event directory, EventUri source watches derived directory, missing rig/tie-offs parents created

### Phase 5: Loom Repository and Config Event Handler — Remove Intent

Remove `Intent` from `FileSystemLoomRepository`, `ConfigEventHandler`, and event source mapping.

- [ ] In `FileSystemLoomRepository::scan_knot_files()`, use `KnotFile.strand_source` to construct `Knot`
- [ ] In `ConfigEventHandler::handle_knot_added/modified()`, use `strand_source` and `event_description`
- [ ] In `event_source.rs` `map_rig_event()` and `map_loom_event()`, construct `Knot` from `KnotFile` with `strand_source` and `event_description` (remove `listens_for` field)
- [ ] **Tests**: Scan loom with EventUri knot, scan loom with Filesystem knot, config event handler registers EventUri knot, config event handler registers Filesystem knot

### Phase 6: Event Metadata and Tie-Off Integration — Derive from strand_source

Update `process_strand` event metadata extraction and `AgentEvent` handling.

- [ ] `AgentEvent` struct: `target_knot` field removed — the producing knot is always known from the tie-off context (ProcessStrand has the knot ID). When parsing, fill `target_knot` from the knot that produced the tie-off.
- [ ] When processing a strand that came from an event dispatch directory, `extract_event_metadata()` should still work (reads frontmatter of the event file — unchanged)
- [ ] `EventMetadata` entity and its serialisation are unchanged
- [ ] Tie-off append with `event_metadata` is unchanged
- [ ] Ensure `StrandSource::EventUri` knots produce correct `EventMetadata` in their tie-offs
- [ ] **Tests**: Event-triggered consumer knot produces tie-off with event metadata, Filesystem knot produces tie-off without event metadata, partial event metadata preserved, `AgentEvent` target_knot derived from producing knot

### Phase 7: Skills, Docs, Domain Glossary — Update for new model

Update all documentation and agent skills to reflect the new `strand-source` / `event:` URI model.

- [ ] `domain-glossary.md`: Update Knot definition (replace `listens-for` with `strand-source` + `event-description`), update Term Relationships diagram, add `StrandSource` term
- [ ] `.agents/skills/knot-create/SKILL.md`: Update knot creation examples to use `event:` URI and `event-description`
- [ ] `.agents/skills/knot-update/SKILL.md`: Add migration entry for this version change
- [ ] `project/plans/intent-based-event-routing.md`: Add note about superseding design
- [ ] **Tests**: None (documentation only)

### Phase 8: Cleanup and Verification — Remove dead code, full test pass

Remove `Intent` struct, `default_listens_for()`, `matches_intent()`, and any remaining `listens_for` references. Full compile and test pass.

- [ ] Remove `Intent` struct from `events.rs` (if not already removed in Phase 3)
- [ ] Remove `matches_intent()` function
- [ ] Remove `default_listens_for()` function
- [ ] Remove all `listens_for` fields from entities, files, test fixtures
- [ ] `cargo clippy -- -D warnings` — clean
- [ ] `cargo test` — all tests pass
- [ ] **Tests**: All existing tests migrated, no dead code

### Phase 9: Construction Consolidation — KnotFile → Knot factory, test builder

Centralise `Knot` construction so that adding or removing fields only requires changes in one place, not every construction site across the codebase.

**Problem**: `Knot` is constructed with struct literals at ~18 sites (3 production, ~15 tests). Every field change ripples through all of them. `KnotFile` and `Knot` are near-identical structs — field-by-field mapping is duplicated in `loom_repository.rs` and twice in `event_source.rs`.

**Solution**: One canonical `KnotFile → Knot` factory, one test builder.

- [ ] **`impl From<KnotFile> for Knot`** on `Knot` — the canonical mapping. `loom_repository.rs::knot_from_file()` and both `event_source.rs` event mappers call `Knot::from(file)` instead of manual field copying
- [ ] **`Knot::with_strand_dir(&self, dir: PathBuf) -> Self`** — returns a clone with the strand dir resolved (used by `loom_repository.rs` scan for path resolution)
- [ ] **`KnotBuilder`** in `test_fixtures.rs` — builder with defaults: only `id` is required; everything else defaults (`agent_profile_ref: "fast"`, `strand_source: Filesystem("strands")`, `event_description: None`, etc.). Provides fluent setters for any field
- [ ] **Consolidate `event_source.rs`** — extract shared helper from `map_rig_event()` / `map_loom_event()` for the common "parse knot file → build Knot → branch Create/Modify" logic
- [ ] **Remove `knot_from_file()`** from `loom_repository.rs` — replaced by `Knot::from()`
- [ ] **Migrate test sites** — replace `Knot { ... }` struct literals with `KnotBuilder::new(id).build()` or `build_knot(id)` across all test modules
- [ ] **Tests**: No new behaviour — verify `From<KnotFile>` produces identical `Knot` to manual mapping, `KnotBuilder` round-trips all fields, `Knot::with_strand_dir()` resolves correctly

## Notes

### StrandSource URI Format

```
event:<producer-knot-id>:<EventId>
```

Three colon-separated parts. No escaping needed — knot IDs are kebab-case slugs, event IDs are PascalCase identifiers. Ambiguity with filesystem paths is impossible because no filesystem path starts with `event:`.

### event-description is Optional

The `event-description` field provides the semantic contract injected into the producer's prompt. When present, `build_listener_context()` includes it. When absent, it injects a generic message:

> Knot `{consumer_id}` is listening for event `{event_id}`. If this occurs, emit a structured event block in your tie-off.

This allows minimal event consumers (where payload structure is not critical) to omit the description.

### Why Not Keep Intent?

`Intent` was designed as a subscriber declaration. `StrandSource::EventUri` is the same information — producer knot + event ID — but expressed as *where this knot gets its input from* rather than *what this knot wants to hear about*. This is conceptually simpler: one field, one direction, one watcher.

The `event-description` field survives because it serves a different purpose: it tells the *producer* what to emit. It's not about routing; it's about producer prompt injection. Keeping it as a simple optional string is sufficient.

### Backward Compatibility

Knot files with `listens-for` in their frontmatter will parse successfully with an unknown-property warning (`listens-for` is no longer a recognised key). The knot will not have event subscriptions unless it also has a valid `strand-dir` (path or URI). This is the same migration pattern used for `tie-off-dir` removal.

### What This Does NOT Cover

- Event expiration or TTL (events persist until manually cleaned)
- Priority or ordering of event delivery (FIFO by creation time)
- Retry on dispatch failure (failed dispatch is logged, not retried)
- Cross-rig event routing (events stay within a single rig)
