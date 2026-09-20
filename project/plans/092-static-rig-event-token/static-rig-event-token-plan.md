# Plan 092: Static Engine Token for Rig-Scoped System Events, with Zero-Consumer Diagnostics

## Related Plans

Amends the rig-level subscription form introduced by
[082 System Event Subscriptions](../082-system-event-subscriptions/system-event-subscriptions-plan.md)
(`event:<rig-id>:<EventId>` → canonical `event:knot:<EventId>`, old form
kept as deprecated), and closes the silent-zero-consumer gap observed
while debugging a live rig (evidence below). Complements
[091 Per-Event Enforcement](../091-per-event-enforcement/per-event-enforcement-plan.md),
which fixed the producer side of the same class of silent gap (missing
event blocks); this plan fixes the subscriber side for rig-scoped
system events.

## Problem

Two related failures, both observed on the `rust-core-4` rig on
2026-09-20 with Knot 0.49.0:

**1. The rig-id subscription token encodes a deployment detail.**
Rig-scoped system events match subscriptions whose producer token equals
the rig directory basename (`SystemEventEmitter::rig_id()` =
`rig_dir.file_name()`). The rig was built from a `software-factory-rig`
template, but its rig directory is named `rig/` — so the orchestrator
knot's subscription `strand-dir: "event:software-factory-rig:QueueIdle"`
never matched. `resolve_rig_event("rig")` returned `None`, the emitter
found zero consumers, and `emit` returned `Ok([])` — no event file, no
error, no log line. `QueueIdle` fired three times (09:09, 09:15,
13:58); the orchestrator never ran. The `QueueIdle/` watch directory
existed and was watched (auto-created by `ensure_event_uri_watch` at
boot) — a watcher sitting on a directory nothing ever writes to.

**2. A zero-consumer dispatch is invisible.** The pipeline logs only on
`Err` (`[pipeline] QueueIdle dispatch failed: …`). A dispatch that
succeeds with zero matches — the *interesting* failure mode — produces
nothing. The bug required a source-code debug session to find.

**Why the rig name earns no place in the token.** Knot runs one
process per rig with no IPC between running rigs. The emitter scans only
its own rig's `LoomStore` and dispatches only into its own rig's
runtime root (`tie-offs/<rig-basename>/…`); the watch is registered on
exactly that path. The token therefore selects between nothing — there
is no cross-rig surface it could discriminate on. It contributes only
fragility: the rig id is an accident of directory naming, so any rename
(or a template built under another name) silently kills every rig-scoped
subscription. A constant token removes the failure class entirely, and
the process boundary already guarantees the scoping the token was
purported to provide.

## Target

- **Rig-scoped system events subscribe by a static engine token:**
  `event:knot:<EventId>` (e.g. `event:knot:QueueIdle`). The token means
  "the Knot engine" — the producer of every system event — and is
  invariant under rig-directory renames and template reuse.
- **The rig-name form is deprecated, not broken:**
  `event:<rig-id>:<EventId>` keeps working in this release; removal is a
  later breaking change (D2).
- **A rig-scoped dispatch that matches zero consumers is loud:** one
  `[KNOT][SYSTEM]` service-log line per such dispatch, naming near-miss
  subscriptions when a knot subscribes to the same event id with a
  non-matching producer token (the rename-mismatch signature).

## Decisions

- **D1 — Token: `knot`.** The reserved producer token for rig-scoped
  system events is the literal `knot`. Rationale: it names the actual
  producer (the engine), reads well ("Knot says the queue is idle"), and
  is unambiguous *in matching* because scope selects the resolver —
  rig-scoped emits only ever call `resolve_rig_event`, where `knot`
  means the engine. A knot actually named `knot` emitting agent events
  resolves through `resolve_for_producer`, where `event:knot:X`
  correctly means "the knot named knot"; the two readings never cross
  (pinned by a regression test). Alternatives rejected: `rig` (looks
  like it should be a variable rig id — the exact confusion that caused
  the bug) and `engine` (less discoverable than the product name).
- **D2 — Transitional acceptance, documented deprecation.**
  `resolve_rig_event` accepts, in order: `*` (wildcard), `knot`
  (canonical), `target == rig_id` (deprecated). The rig-name form is
  marked deprecated in all docs and skills in this release; it is
  removed in a later breaking release, not this one. A rig whose
  basename is literally `knot` is unaffected (both branches yield the
  same single match).
- **D3 — Wildcard unchanged.** `event:*:<EventId>` continues to match
  rig-scoped dispatches exactly as today (plan 082 semantics).
- **D4 — Event-file provenance unchanged.** The `target-knot:`
  frontmatter of a dispatched system event file keeps carrying the
  *actual* rig id (the basename), regardless of the subscription form.
  Only matching changes; the file remains attributable without path
  context. (`emit` already builds the producer token from
  `self.rig_id()`, independent of the consumer's subscription.)
- **D5 — Diagnostic scope: rig-scoped emits only.** Zero-consumer
  logging applies to `EventScope::Rig` dispatches only. Zero consumers
  is the *normal* state for knot- and loom-scoped events (most rigs
  subscribe to no failure/monitoring events); logging those would be
  per-run noise. Rig-scoped emits are rare and bounded (one `QueueIdle`
  per burst, 500 ms after the queue drains), so the volume is at most
  one line per burst — and only when something is wrong.
- **D6 — Near-miss detection is the high-signal form.** When a
  rig-scoped emit matches zero consumers, the emitter also reports every
  knot whose `strand-dir` is an event URI with the *same event id* but a
  non-matching producer token — e.g. `near-miss: pipeline-on-queue-idle
  (event:software-factory-rig:QueueIdle)`. That line names the rename
  mismatch directly; a plain `0 consumers matched` line covers the no
  subscription at all case. New service-log tag `[KNOT][SYSTEM]`
  (low-volume diagnostic, consistent with the existing `[KNOT][TAG]`
  line family).

## Design

### Phase 1 — Domain: static token in `resolve_rig_event`
(`src/domain/value_objects.rs`)

- Add `pub const RIG_EVENT_ENGINE_TOKEN: &str = "knot";` next to
  `EventSubscription` (documented: reserved producer token for
  rig-scoped system events; the Knot engine).
- `StrandSource::resolve_rig_event(rig_id)` — matching order:
  1. `target == "*"` → `Wildcard` (unchanged)
  2. `target == RIG_EVENT_ENGINE_TOKEN` → `RigLevel` (canonical, new)
  3. `target == rig_id` → `RigLevel` (deprecated transitional form)
  4. else `None` (unchanged)
- `from_str` parsing is unchanged (`knot` is a valid two-part token);
  the `strand-dir` format doc comment (~line 1007) and `from_str` docs
  gain the reserved-token note: rig-scoped subscriptions use
  `event:knot:<EventId>`; `event:<rig-id>:<EventId>` is deprecated.
- The "collision note" in the plan-082 docs (a knot named after the rig
  is harmless) becomes moot and is retired from the doc comment — with
  a static token there is no rig id to collide with; the new relevant
  note is the `knot`-named-knot cross-scope guard (test below).

Unit tests (`value_objects.rs`):
- `resolve_rig_event` static token → `RigLevel` (new)
- rig-name form still → `RigLevel` (existing test kept — pins the
  transitional guarantee)
- wildcard / foreign-token behaviour (existing, unchanged)
- **Cross-scope guard (new):** for a knot named `knot`,
  `resolve_for_producer("knot", …)` resolves `event:knot:<EventId>` as
  knot-level — the engine token must not leak into knot-scoped matching.

### Phase 2 — Emitter: zero-consumer diagnostic
(`src/application/usecases/system_event_emitter.rs`,
`src/adapters/logging.rs`)

- `adapters/logging.rs`: new helper (application use cases already call
  `adapters::logging` — `mod_watchers.rs` precedent):
  ```rust
  /// Log a system-event dispatch diagnostic. Low volume: emitted only
  /// when a rig-scoped system event matched zero consumers.
  pub fn log_system_event(event_id: &str, rig_id: &str, detail: &str) {
      eprintln!(
          "[{}] [KNOT][SYSTEM] event={event_id} rig={rig_id} — {detail}",
          format_timestamp()
      );
  }
  ```
- `SystemEventEmitter::emit`, `EventScope::Rig` arm: while scanning for
  matches, also collect **near misses** — knots whose `strand_source`
  is an `EventUri` with `event_id() == event_id` but for which
  `resolve_rig_event` returned `None` (record knot id + raw producer
  token). After the scan:
  - `matches` empty **and** near-misses non-empty →
    `log_system_event(event_id, &rig, "0 consumers matched; near-miss
    subscription(s): <knot> (event:<token>:<EventId>)[, …]")`
  - `matches` empty **and** no near-misses →
    `log_system_event(event_id, &rig, "0 consumers matched")`
  - `matches` non-empty → no line (normal dispatch; the event file is
    the observable).
- No change to `server.rs` call sites — the diagnostic lives in the
  emitter, so every rig-scoped emit site (today `QueueIdle`, future
  rig-scoped events) is covered without touching the pipeline.
- Module doc line 20 updated: `EventScope::Rig — resolve_rig_event`
  (`knot` (canonical) / `<rig-id>` (deprecated) / `*`).

Unit tests (`system_event_emitter.rs`):
- rig-scoped emit reaches a `event:knot:QueueIdle` consumer (new —
  mirrors the existing `event:dev-rig:QueueIdle` test)
- producer token written for a static-token consumer is still the rig
  basename (D4 — extend `rig_emit_producer_token_is_rig_id` with a
  static-token store)
- deprecated rig-name consumer still reached (existing — kept)
- near-miss store (consumer on `event:other-rig:QueueIdle`): result is
  empty (the `[KNOT][SYSTEM]` line is a stderr diagnostic; behaviour is
  the unchanged zero dispatch)

### Phase 3 — Integration test: rig-scoped QueueIdle end to end
(`tests/system_event_subscriptions.rs`, following the `RigFixture`
harness used by `tests/queue_identity.rs`)

The rig-scoped path is currently unit-tested only (mock dispatcher);
add the missing end-to-end coverage while the machinery is being
touched:

1. **Static-token consumer fires.** Fixture rig with a consumer knot
   (`strand-dir: "event:knot:QueueIdle"`, instructions that produce a
   tie-off). Enqueue a strand for any knot, wait for the queue to drain
   (the `queue_drained` pattern from `tests/queue_identity.rs`), and
   assert: an `event-*.md` file exists in
   `tie-offs/<rig>/<consumer-loom>/QueueIdle/` and the consumer knot
   ran (tie-off present / expected service-log lines).
2. **Near-miss is loud, file is absent.** A consumer knot subscribing
   `event:<wrong-rig-id>:QueueIdle` (deliberately mismatched basename).
   After drain: no event file for it, and the service log contains a
   `[KNOT][SYSTEM]` line naming the near-miss subscription (grep the
   log, the `tests/consolidated_log.rs` pattern).

### Phase 4 — Docs and skills

- `docs/concepts.md` (§ system events, lines ~424–432): rig-level form
  → `event:knot:<EventId>` (canonical); rig-name form marked
  deprecated (still accepted).
- `docs/release-notes.md`: new `v0.50.0` section (summary of D1–D6,
  the diagnostic line format, deprecation notice).
- `.agents/skills/knot-create/SKILL.md`: the `strand-dir` table row
  (~383), the URI-forms list (~433–435), the subscription-context note
  (~580), and the system-event catalog (`QueueIdle` row, ~633–640) —
  rig-level form becomes `event:knot:<EventId>`; rig-name form
  deprecated.
- `.agents/skills/knot-analyst/SKILL.md`: add the `[KNOT][SYSTEM]` line
  to the service-log vocabulary (one line).
- `.agents/skills/knot-update/SKILL.md`: `v0.50.0` changelog entry —
  no migration required; new canonical form; old form deprecated;
  recommendation to move rig-scoped subscriptions to `event:knot:`.
- Plan 082 file: one-line supersession note on its rig-level form
  ("amended by Plan 092: canonical form is `event:knot:<EventId>`; the
  rig-name form is deprecated").
- Deploy the updated skills to `~/.agents/` (master + skills-library)
  with the diff verification from `AGENTS.md`.
- Operator note (out of repo): the `rust-core-4` orchestrator should be
  migrated to `event:knot:QueueIdle` (and its stale
  `tie-offs/software-factory-rig/…` path references fixed) — not done
  from this repo.

### Phase 5 — Index, version, install

- `project/plans/master-plan.md`: add row 92 (🟡 In Progress →
  ✅ Complete on completion), update the `Last Updated` header.
- Bump `Cargo.toml` to `0.50.0` (additive + diagnostic only; nothing
  removed — minor).
- `cargo install --path .` per `AGENTS.md`.

## Verification

- `cargo test` (full suite, including the new unit and integration
  tests) and `cargo clippy` clean.
- **No live rig runs in this repository** (`AGENTS.md`): the
  integration tests run the binary against fixture rigs via the
  mock-CLI harness, which is the sanctioned path.

## Migration / Deprecation

- 0.50.0 accepts both `event:knot:<EventId>` and
  `event:<rig-id>:<EventId>`; existing rigs keep working untouched.
- All authoring surfaces (skills, docs, catalog) present only the
  static form as canonical, with a deprecation pointer on the old form.
- Removal of the rig-name form is reserved for a later **breaking**
  release; this plan commits no removal timeline.

## Out of Scope

- Extending the zero-consumer diagnostic to knot- and loom-scoped
  emits (D5: normal zero-consumer state there; noise if logged).
- Changing the `target-knot:` frontmatter semantics (D4 keeps
  provenance as the actual rig id).
- Multi-rig-per-process or IPC — the one-process-per-rig model is the
  premise, not something this plan changes.
- Removing the deprecated form (later breaking release).

## Implementation Status: ✅ Complete (2026-09-21) — released in v0.50.0

### Implementation Log

- **Phase 1 (domain)** — `RIG_EVENT_ENGINE_TOKEN` ("knot") const added
  next to `EventSubscription` with the scope-locality rationale
  documented; `resolve_rig_event` gains the static-token branch
  between wildcard and the deprecated rig-name branch; `RigLevel`
  variant docs amended. Tests: `resolve_rig_event_static_engine_token`
  (canonical form matches any rig; rig literally named `knot`
  unaffected), `engine_token_does_not_leak_into_knot_scoped_matching`
  (a knot named `knot` resolves `event:knot:X` as knot-level via
  `resolve_for_producer`; a producer knot named `knot` is an ordinary
  producer for other subscribers).
- **Phase 2 (emitter + diagnostic)** — `adapters::logging` gains
  `log_system_event` (`[KNOT][SYSTEM] event=… rig=… — …`); the
  `EventScope::Rig` arm of `SystemEventEmitter::emit` collects near
  misses (event URIs on the same event id with a non-matching producer
  token) and logs the diagnostic when matches are empty (near-miss
  detail when present). Tests: `rig_emit_static_engine_token_consumer`
  (static + deprecated forms dispatch side by side),
  `rig_emit_producer_token_is_rig_id_for_static_consumer` (D4:
  frontmatter carries the actual rig id),
  `rig_emit_near_miss_does_not_dispatch` (behaviour unchanged — zero
  dispatch).
- **Phase 3 (integration)** — `tests/system_event_subscriptions.rs`
  gains a binary-level section (the `QueueIdle` emit site is the
  service's pipeline loop, so the in-process `ProcessStrand` harness
  cannot cover it): `queue_idle_reaches_static_engine_token_consumer`
  (burst → drain → `event-*.md` in the consumer's `QueueIdle/` dir
  with `target-knot: dev-rig`, consumer knot runs, no `[KNOT][SYSTEM]`
  line — positive control) and
  `queue_idle_near_miss_is_loud_and_dispatches_nothing` (no event
  file; the service log carries the `[KNOT][SYSTEM]` line naming the
  near-miss subscription).
- **Phase 4 (docs + skills)** — `docs/concepts.md` rig-level form
  amended; `knot-create` skill: frontmatter table, URI-forms list,
  subscription-context note, system-event catalog (all to the static
  form, deprecated alias noted; catalog gains the zero-consumer
  diagnostic paragraph); `knot-analyst` skill: `[KNOT][SYSTEM]` row in
  the service-log signal table; `knot-update` skill: v0.50.0 changelog
  entry (optional migration); plan 082 supersession note. Skills
  deployed to `~/.agents/skills-library/` with diff verification
  (knot-create 5.9.0, knot-analyst 1.8.0, knot-update 1.23.0).
- **Phase 5 (index, version, install)** — `Cargo.toml` 0.50.0; full
  `cargo test` green (1106 lib unit + all integration suites, zero
  failures); `cargo clippy --all-targets` introduces no new warnings
  (207 pre-existing at baseline); `cargo install --path .` replaced
  the 0.49.0 binary (verified `knot 0.50.0`); release notes v0.50.0
  section added; master plan row 92 marked complete.

**Verification note:** no live rig runs were performed in this
repository (per `AGENTS.md`) — the binary-level integration tests
spawn the compiled service against fixture rigs with a mock agent
CLI, which is the sanctioned path.
