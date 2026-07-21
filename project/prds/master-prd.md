# Master PRD — Feature Index

> **Last Updated:** 2026-07-20

## How to Add a PRD

Each PRD file must contain a title (e.g. `# PRD: Feature Name`).

To add it to this index:

1. Add a row to the table below: link, status, date.
2. Optionally add a one-line summary below the table.

**Ordering:** PRDs are ordered by creation date, latest first. Unknown dates (`—`) appear last.

---

## PRD Index

| PRD | Status | Created |
|-----|--------|---------|
| [Input Reconciliation — Automatic Ledger for Completeness Checking](prd-input-reconciliation.md) | 🔵 Open | 2026-07-20 |
| [Spurious Delete Suppression — Burst Event Deduplication](prd-spurious-delete-suppression.md) | 🔵 Open | 2026-07-16 |
| [Persistent Events — Disk-Backed Event Queue](prd-persistent-events.md) | 🔵 Open | 2026-07-15 |
| [Tie-Off Event Enforcement](prd-tie-off-event-enforcement.md) | 🔵 Open | 2026-07-14 |
| [Demand Control — Concurrency, Throughput and Service Tuning](prd-demand-control.md) | 🔵 Open | 2026-06-26 |
| [System Reliability — Messaging Control, Replay and Rollback](prd-system-reliability.md) | 🔵 Open | 2026-06-09 |
| [Knot Skills — AI-Driven Configuration via Skills](prd-knot-skills.md) | ✅ Complete | 2026-06-04 |
| [AI-Driven File Generation from Loom Events](prd-ai-driven-file-generation.md) | ✅ Complete | 2026-06-03 |

---

## PRD Summaries

### Input Reconciliation — Automatic Ledger for Completeness Checking

**Status:** 🔵 Open
**Created:** 2026-07-20
**Summary:** Automatic input reconciliation via per-loom ledger files — agents declare what documents they process, Knot diffs against current state, injects gaps into prompts, and updates ledgers after successful turns.

Full details in [prd-input-reconciliation.md](prd-input-reconciliation.md).

### Spurious Delete Suppression — Burst Event Deduplication

**Status:** 🔵 Open
**Created:** 2026-07-16
**Summary:** Suppress spurious DELETE strand events caused by atomic save patterns (truncate+write) so the agent pipeline is not invoked for transient file deletions that are immediately followed by CREATE or MODIFY.

Full details in [prd-spurious-delete-suppression.md](prd-spurious-delete-suppression.md).

### Persistent Events — Disk-Backed Event Queue

**Status:** 🔵 Open
**Created:** 2026-07-15
**Summary:** Persist strand events to `rig/events/` as JSON files so pending work survives restarts, and expose HTTP endpoints to list and delete queued events.

Full details in [prd-persistent-events.md](prd-persistent-events.md).

### Tie-Off Event Enforcement

**Status:** 🔵 Open
**Created:** 2026-07-14
**Summary:** Detect when agents instructed to emit tie-off events fail to do so, log the failure, and re-enter the session with a follow-up prompt to remind the agent to provide events.

Full details in [prd-tie-off-event-enforcement.md](prd-tie-off-event-enforcement.md).

### Demand Control — Concurrency, Throughput and Service Tuning

**Status:** 🔵 Open
**Created:** 2026-06-26
**Summary:** Max parallel agent invocations, invocation performance visibility (recent N durations), token usage capture, and global rig configuration to tune outgoing demand on external AI services.

Full details in [prd-demand-control.md](prd-demand-control.md).

### System Reliability — Messaging Control, Replay and Rollback

**Status:** 🔵 Open
**Created:** 2026-06-09
**Summary:** Rate limiting, concurrency caps, budget/token limits, usage visibility, event replay, and tie-off rollback to protect providers and control cost.

Full details in [prd-system-reliability.md](prd-system-reliability.md).

### Knot Skills — AI-Driven Configuration via Skills

**Status:** ✅ Complete
**Created:** 2026-06-04
**Completed:** 2026-06-04
**Summary:** AI skills (knot-init, knots-and-looms, knot-inspect) that configure Knot through natural language via its HTTP API, backed by an auto-generated Swagger UI.

Full details in [prd-knot-skills.md](prd-knot-skills.md).

### AI-Driven File Generation from Loom Events

**Status:** ✅ Complete
**Created:** 2026-06-03
**Completed:** 2026-06-04
**Summary:** Watch a configured rig for file events and use AI to generate corresponding output files in a target directory based on a user-defined goal.

Full details in [prd-ai-driven-file-generation.md](prd-ai-driven-file-generation.md).
