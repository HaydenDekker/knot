# Phase 6: Observability — Structured tie-off entries for events

**Plan:** [Intent-Based Event Routing](intent-based-event-routing-plan.md)

## Checklist
- [x] Add `EventMetadata` struct to domain entities with `event_id`, `source_knot`, `original_strand` fields
  - Derives `Debug`, `Clone`, `PartialEq`, `Eq`, `Serialize`, `Deserialize`, `Default`
  - `is_some()` / `is_none()` convenience methods
  - Serde: `skip_serializing_if = "is_none"` on the struct level, `skip_serializing_if = "Option::is_none"` on fields, `rename` on `source-knot` and `original-strand`
- [x] Add `event_metadata: EventMetadata` field to `TieOff` entity
  - Serde: `default`, `skip_serializing_if = "EventMetadata::is_none"`
- [x] Add `extract_event_metadata(strand_path)` helper in `process_strand.rs`
  - Quick check: filename starts with `event-`
  - Reads file, parses YAML frontmatter
  - Returns `Some(EventMetadata)` with `event_id` and `source_knot` (from `event-id` and `target-knot` frontmatter)
  - Returns `None` for non-event files, missing files, or files without matching frontmatter
- [x] Add `parse_yaml_frontmatter(content)` helper in `process_strand.rs`
  - Expects `---` delimited frontmatter at start of file
  - Parses simple key-value pairs (splits on first `:`)
  - Trims whitespace, skips empty lines
  - Returns `Some(HashMap)` or `None` (no frontmatter / empty)
- [x] Update `FileSystemTieOffSink::append()` to include structured event metadata
  - After standard header (`## knot triggered by event strand`, `Timestamp:`), appends:
    - `event: {event_id}` when set
    - `source: {source_knot}` when set
    - `original_strand: {original_strand}` when set
  - Only fields that are `Some` are written (partial metadata supported)
- [x] Update `ProcessStrand::execute()` to extract event metadata before writing tie-off
  - Calls `extract_event_metadata(&strand_path)` before tie-off construction
  - Populates `event_metadata` field on `TieOff` entity
  - Non-event strands get default (empty) `EventMetadata` — no extra output
- [x] Update all `TieOff` construction sites across codebase
  - `src/domain/entities.rs`: 4 test sites
  - `src/adapters/outbound/tieoff_sink.rs`: 7 test sites
  - `src/application/ports.rs`: 1 test site
  - `src/application/usecases/process_strand.rs`: 2 construction sites (error + success)
  - `tests/adapters.rs`: 4 test sites
- [x] **Tests**: 25 new unit tests
  - 6 tests for `EventMetadata` (default, is_some, is_none, serialisation roundtrip, omits null fields, renamed keys, deserialisation with missing fields, full deserialisation)
  - 6 tests for `parse_yaml_frontmatter` (valid basic, payload fields, no frontmatter, empty frontmatter, whitespace trimming, skips empty lines)
  - 7 tests for `extract_event_metadata` (event file extraction, regular file returns None, no frontmatter returns None, missing file returns None, no event-id/target-knot returns None, partial fields, original-strand support)
  - 3 tests for tie-off append with event metadata (all fields present, no extra fields for normal strands, partial metadata shows only set fields)
  - 3 tests already existed in `pi_stdio.rs` for event metadata passthrough

## Deviations
- `EventMetadata` is stored as `EventMetadata` (not `Option<EventMetadata>`) on the `TieOff` entity — uses `is_none()` check and `Default` instead. This is consistent with the existing pattern of optional-but-struct fields and allows `skip_serializing_if` to work cleanly.
- `extract_event_metadata` is a standalone function (not a method on `StrandPath`) because it performs filesystem I/O (`std::fs::read_to_string`), which is inappropriate for a domain value object.

## Discoveries
- The event file frontmatter already contains `event-id` and `target-knot` — these map directly to `event_id` and `source_knot` in `EventMetadata`.
- The `original-strand` field in event file frontmatter is optional — when the dispatch layer doesn't propagate it, `original_strand` in the consumer's tie-off will be `None` (not written).
- The YAML frontmatter parser only needs simple key-value parsing — event files never contain nested YAML, so a split-on-first-colon approach is sufficient.

## Notes
- Event metadata in tie-offs enables counting a2a messages from tie-off content alone (grep for `event:` lines).
- The structured metadata block is written between the standard header and the `---` delimiter, so existing parsers that split on `---` still work.
- All existing tie-off entries (without event metadata) remain valid — the metadata block is only appended when `EventMetadata` fields are set.
