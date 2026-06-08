# Master PRD — Feature Index

> **Last Updated:** 2026-06-09

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
| [System Reliability — Messaging Control, Replay and Rollback](prd-system-reliability.md) | 🔵 Open | 2026-06-09 |
| [Knot Skills — AI-Driven Configuration via Skills](prd-knot-skills.md) | ✅ Complete | 2026-06-04 |
| [AI-Driven File Generation from Loom Events](prd-ai-driven-file-generation.md) | ✅ Complete | 2026-06-03 |

---

## PRD Summaries

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
