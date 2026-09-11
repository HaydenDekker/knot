# Phase 4: Bugfix — Block-Scalar Front-Matter Round-Trip (Background Accumulation)

**Plan:** [Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget](087-event-source-continuation-and-queue-budget-plan.md)

## Context

First live run of the event-watermark continuation chain (2026-09-10,
`pwa-todo-2` rig, `phase-implementer` / `PhaseReady`) exposed a regression
in the "accumulated, hop-labelled (086)" `background-additional`. The hop-2
continuation arrived with an empty-looking front-matter block-scalar (lone
`|` markers around an empty `[hop 2]` label) and a bare-`|`
`## Next Task Context`. The background did **not** accumulate across hops:
both halves of the invariant — pass the incoming accumulation **through**
and **append** the agent's new input — were lost.

## Root cause

Both front-matter parsers are naive line-based `key: value` splitters with
no YAML block-scalar (`|` / `>`) support:

1. **`tieoff_parser::parse_frontmatter`** (via `parse_kv_line`) — parses the
   agent's ```markdown tie-off block into `AgentEvent.payload`. For
   `next-task-context: |` / `background-additional: |` it records the value
   as the literal `"|"` and **drops the indented body**; indented body lines
   that contain a `:` are even mis-parsed as spurious top-level keys. →
   kills the **append** (the agent's new contribution + operational brief).
2. **`strand_event_metadata::parse_yaml_frontmatter`** — reads the *prior*
   continuation file's front-matter to recover `incoming_bg`. The same naive
   split yields `background-additional = "|"`. → kills the **pass-through**
   (the accumulated history from earlier hops).

`dispatch_self_continuation` therefore writes `[hop N]` labels with empty
bodies (`|` / `[hop 2]` / `|`, as observed). The pre-existing test
(`extract_agent_events_unclosed_trailing_fence_still_parsed`) exercises
`|`-block fields but only asserts event *recovery* (event_id + occurred),
never the *captured value* — which is why this slipped through.

## Fix

Make both parsers block-scalar-aware via a single shared implementation:

1. **`tieoff_parser::parse_frontmatter`** — block-scalar-aware, made
   **public**, and operating on **raw (untrimmed)** lines. On a `key: |` /
   `key: >` line (optional `+`/`-` chomping indicator), collect the
   following indented-or-blank lines as the body, de-indent by the common
   leading whitespace, and join with `\n`. All other keys keep the existing
   `key: value` behaviour.
2. **`parse_event_block`** — pass **raw** front-matter lines to
   `parse_frontmatter` (drop the `.map(|s| s.trim())`) so the block-scalar
   body's indentation survives to be recognised.
3. **`parse_yaml_frontmatter`** — delegate its line-parsing to the shared
   `tieoff_parser::parse_frontmatter`; keep its `Option` semantics (still
   `None` for no/empty front-matter). Simple scalar keys
   (`continuations`, `budget-secs`, `batch-start-epoch`, `event-id`, …) are
   unaffected.

## Checklist

- [x] **`parse_frontmatter` captures a multi-line `next-task-context: |`** verbatim (de-indented) — *append* (the operational brief) — `parse_frontmatter_block_scalar_captured_verbatim` (`tieoff_parser.rs`)
- [x] **`parse_frontmatter` captures a multi-line `background-additional: |`** verbatim (de-indented) — *append* (the agent's new facts) — same test + `parse_frontmatter_block_scalar_folding_and_chomp_markers` (`tieoff_parser.rs`)
- [x] **`parse_yaml_frontmatter` round-trips a continuation file's `background-additional: |`** — *pass-through* (the incoming accumulation) — `parse_yaml_frontmatter_block_scalar_round_trip` (`strand_event_metadata.rs`)
- [x] **End-to-end `dispatch_self_continuation`**: hop N+1's file `## Accumulated Background` carries hop N's body **and** the new `[hop N+1]` line — the full append-and-pass-through invariant — `dispatch_self_continuation_accumulates_background_across_hops` (`process_strand_helpers.rs`)
- [x] Existing `extract_agent_events_*` tests still green; the `|`-block test (`extract_agent_events_unclosed_trailing_fence_still_parsed`) is strengthened to also assert the captured block-scalar value — no regression
- [x] `cargo clippy --all-targets` — zero new warning bodies (normalised before/after warning-body multiset is **identical**)
- [x] `cargo test --no-fail-fast` — full suite green: **1353 passed, 0 failed** (baseline 1349 + 4 new tests)

## Deviations

- **One existing test's fixture changed, not its assertion.**
  `parse_yaml_frontmatter_whitespace_trimming` fed a top-level key with a
  **leading space** (` target-knot: plan-creator `), which the old lenient
  parser tolerated by trimming every line first. The new parser is
  YAML-accurate: a top-level key sits at **column 0**, and an indented line
  is block-scalar *body*, so an indented "key" is no longer parsed as a key.
  The test's **intent** (values are trimmed) is preserved; the fixture was
  corrected to a column-0 key. This is the one behaviour change with a user-
  facing surface: a malformed agent tie-off that indents a top-level key is
  now dropped for that key instead of recovered (Knot-written continuation
  and event files are always column-0, so this never bites in practice).
- **Block-scalar folding (`>`) stores the literal body, not folded text.**
  v1 treats `>` like `|` (de-indented, newlines preserved) rather than
  collapsing internal newlines to spaces. The agents emit `|`, so this is
  untested-by-practice but the conservative, lossless choice (see Notes).

## Discoveries

- **The read side was the only broken half.** `dispatch_self_continuation`
  already writes a correct `background-additional: |` block scalar
  (2-space-indented body); the failure was entirely that the two readers
  (`tieoff_parser::parse_frontmatter` for the agent's tie-off block,
  `strand_event_metadata::parse_yaml_frontmatter` for the prior continuation
  file) collapsed a block scalar to the literal `"|"`. The fix is read-side
  only: one shared block-scalar-aware `parse_frontmatter` (made public),
  `parse_event_block` now passes **raw** (untrimmed) front-matter lines so the
  body indentation survives, and `parse_yaml_frontmatter` delegates to the
  shared parser.
- **The pre-existing `|`-block test was the mask.**
  `extract_agent_events_unclosed_trailing_fence_still_parsed` used
  `|`-block fields but asserted only event *recovery* (event_id + occurred),
  never the *captured value* — so the collapse to `"|"` passed. It is now
  strengthened to assert both block-scalar bodies verbatim.
- **End-to-end test discriminates both halves at once.**
  `dispatch_self_continuation_accumulates_background_across_hops` runs hop 1
  (initial trigger) then hop 2 (fed by hop 1's own continuation file, so
  `incoming_bg` is read back through the fixed `parse_yaml_frontmatter`) and
  asserts hop 2's `## Accumulated Background` contains hop 1's pass-through
  body **and** the new `[hop 2]` line, in order. Fails on either a broken
  append **or** a broken pass-through.

## Notes

- The block-scalar body is indented relative to the column-0 key line; the
  de-indent drops only the **common** leading whitespace, so mixed-indent
  bodies (a sub-bullet under a bullet) keep their relative indentation.
- Block-scalar indicators accepted: `|`, `|-`, `|+`, `>`, `>-`, `>+`.
  YAML folding (`>`) would collapse internal newlines to spaces; v1 stores
  the literal de-indented text for both forms. The agents emit `|`, and a
  literal body is the conservative, lossless choice for
  `next-task-context` (an operational brief where newlines are meaningful).
- `parse_yaml_frontmatter` has two other callers, both unaffected by adding
  block-scalar support: `extract_event_metadata` (reads only `event-id` /
  `target-knot` / `original-strand`, all simple scalars) and
  `read_continuation_stamps` (reads `continuations` / `budget-secs` /
  `batch-start-epoch`, all simple scalars).
- **Release note:** this is a behaviour fix to the continuation
  background-accumulation introduced in v0.44.0 (plan 087) — the accumulated
  `background-additional` now actually carries across hops. No file-format
  change (the on-disk continuation front-matter is unchanged); no migration
  needed. The fix must ship before the next release that relies on
  cross-hop background accumulation.
