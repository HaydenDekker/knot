# ADR-012: Event Files Placed Per Consumer

**Date**: 2026-07-09
**Status**: Accepted

## Context

Intent-based event routing ([Plan 45](../plans/intent-based-event-routing.md)) introduces a mechanism where Knot parses structured events from a producer knot's tie-off and dispatches them to consumer knots. A decision was needed about where the dispatched event file is written on disk.

The key requirement is **selective replay**: a user should be able to re-trigger an event for a specific consumer knot without affecting other consumers of the same event. This matters because:
- A single event may be consumed by multiple knots (fan-out)
- A user may want to replay an event for one knot while leaving others untouched
- Replay is done by touching or modifying the event file, which triggers the filesystem watcher

## Decision

Event files are placed in the **consumer loom's tie-off directory**, under an event-named subdirectory:

```
rig/tie-offs/{consumer-loom-id}/{event-id}/
└── event-{timestamp}.md
```

For example, if `plan-planner` (in `planning-loom`) listens for `PlanCreated` from `plan-reviewer` (in `review-loom`):

```
rig/tie-offs/planning-loom/
├── tie-off-plan-planner.md              ← consumer's own tie-off log
└── PlanCreated/                         ← event folder (named by event-id)
    └── event-2026-07-09T10:00:00Z.md    ← dispatched event
```

If two knots listen for the same event, **each gets its own copy** in its own tie-off directory. The event is duplicated on disk but scoped per consumer.

### Architecture Overview

```
Producer knot runs
  └── writes structured event in its tie-off:
      rig/tie-offs/review-loom/plan-reviewer/
          └── tie-off-plan-reviewer.md
              └── [timestamp] event: PlanCreated ...

Knot parses event from tie-off
  ├── matches against all listens-for intents
  │
  ├── Consumer A: planning-loom/plan-planner (listens for PlanCreated)
  │   └── creates:
  │       rig/tie-offs/planning-loom/PlanCreated/
  │           └── event-2026-07-09T10:00:00Z.md
  │           └── [watcher fires → plan-planner processes]
  │
  └── Consumer B: docs-loom/docs-updater (also listens for PlanCreated)
      └── creates:
          rig/tie-offs/docs-loom/PlanCreated/
              └── event-2026-07-09T10:00:00Z.md
              └── [watcher fires → docs-updater processes]
```

### Implications for Design

- Event files are scoped per **loom**, not per knot. The consumer's `strand-dir` points at the loom's tie-off directory (`../../tie-offs/{loom-id}/`), which contains the knot's own tie-off file and all event subdirectories. Knot must ensure the consumer's own tie-off file (`tie-off-{knot}.md`) is not processed as a strand — handled by existing strand watch filtering.

- A knot listening for multiple event types shares the same loom-level event namespace. Events for different knots in the same loom coexist alongside each other (e.g. `PlanCreated/`, `ReviewComplete/`), and the `strand-dir` watch covers them all.

- Event files are part of the loom's tie-off directory, which is already the derived-state namespace. They are excluded from sharing packages and regenerated on any machine.

- Replay is selective: touching `planning-loom/PlanCreated/event-*.md` re-triggers only the `planning-loom` consumers. Touching `docs-loom/PlanCreated/event-*.md` re-triggers only the `docs-loom` consumers.

### Testing Strategy

- Unit test: verify event file is created in consumer's tie-off directory under the correct event-named subdirectory
- Integration test: two consumers listening for the same event from one producer — verify two separate event files are created in two separate directories
- Integration test: replay by touching one consumer's event file — verify only that consumer re-fires, not the other

## Consequences

### Positive

- **Selective replay** — the primary motivation. Replaying an event affects only the targeted consumer loom.
- **Scoped to loom** — event files live at the loom level, so multiple knots in the same loom share event directories. The producer's tie-off stays clean (only its own tie-off log).
- **Consistent with tie-off layout** — event files are in the same namespace as the consumer loom's tie-offs, already the derived-state directory.

### Negative

- **Event duplication** — if N consumers listen for the same event, the event file is written N times. In practice, fan-out is small (2-4 consumers), and event files are small (structured metadata + brief body).
- **Directory management** — event subdirectories are created lazily on first dispatch. Not a concern — `create_dir_all` handles this idempotently.

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Event file placed in producer's tie-off directory (`rig/tie-offs/{producer-loom}/{producer-knot}/{event-id}/event.md`) | Replay would trigger all downstream consumers. Touching the event file in the producer's directory would re-fire every knot watching it. No mechanism for selective replay without per-consumer copies. |
| Event file placed in consumer's `loom-box` (synthetic strand in loom directory) | `loom-box` lives inside the loom directory, which is watched for config changes. Event files there risk confusion with knot definition files. Tie-off directory is the correct derived-state namespace. |
| Event delivered via in-memory queue only, no persistent file | No replay capability. Events vanish after processing. Users need to be able to re-trigger events for specific knots. |
| Single event file with per-consumer metadata | Would require a central event registry and consumer state tracking. More complex than simple per-consumer file copies. Replay still requires per-consumer triggering. |

## References
- [Plan 45: Intent-Based Event Routing](../plans/intent-based-event-routing.md)
- [ADR-008: Full File-First State](adr-008-full-file-first-state.md) — tie-off directories are derived state
