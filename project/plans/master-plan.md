# Master Plan — Project Index

> **Last Updated:** 2026-10-23 (plan 099 drafted — unified `tie-offs.log` + direct payload-embedded event queueing)
> **Prior:** 2026-10-18 (plan 097 phases 4–5 complete — `queue-busy` dispatch observation + e2e acceptance; released as 0.62.0; phase 6 remains gated on field evidence)
> **Prior:** 2026-10-16 (plans 094–095 complete — the event-enforcement follow-up tie-off is now appended (bugfix) + the `knot-template` skill and per-rig instruction links (hard-coded default + config links); released in v0.59.0)
> **Prior:** 2026-10-02 (plan 093 phase 10 — event-types panel: invocations / consumers / minutes caused per event type, the communication view — v0.58.0)
> **Prior:** 2026-10-02 (plan 093 draft — Rig Graph Visualiser: extract the rig's knot/strand graph into a self-contained offline D3 HTML visualisation, packaged as the `knot-visualise` skill)
> **Prior:** 2026-09-21 (plan 092 complete — static engine token `event:knot:<EventId>` for rig-scoped system events, with zero-consumer diagnostics (`[KNOT][SYSTEM]`); the rig-name form is deprecated, released in v0.50.0)
> **Prior:** 2026-09-12 (plan 089 complete — follow-on phases 8–13 shipped in v0.47.0: settle-based teardown, in-session continuation after a compaction, any-reason interruptions, compaction-aware inactivity window, `TurnContinued` / `RunAbandoned`)
> **Prior:** 2026-09-12 (plan 089 reopened from the 2026-09-11/12 `pwa-todo-3` evidence — threshold compactions still ended the attempt and compaction stalls tripped the inactivity watchdog)
> **Prior:** 2026-09-11 (plan 089 phases 0–7 complete — Interrupted Overflow Compaction: manual compact + session-restart recovery when pi's in-process recovery dies mid-compaction, released in v0.46.0)

## How to Add a Plan

Each plan file must contain a title (e.g. `# Plan: Plan Name`).

To add it to this index:

1. Add a row to the Master Progress Table: number, link, status, date.
2. Optionally add an overview section below with **goal** only. The goal states **what** the plan covers — not why or how. Full details belong in the plan file.

**Ordering:** Plans are ordered by creation date, latest first, within the table. Unknown dates (`—`) appear last.

---

## Purging Old Completed Plans

When updating `master-plan.md`, **remove any plan that is `✅ Complete` and meets this criteria:**

1. **Completed more than 4 weeks ago** — use completion date from the plan file's Implementation Status

Rationale: Once a plan has been complete for a significant period, its status in the index no longer provides active value. The plan file itself (in `project/plans/`) remains as historical documentation. Only the index entry is removed.

**What to remove:**
- The row from the **Master Progress Table**
- Any overview section for this plan

**What to keep:**
- The plan file in `project/plans/` — historical documentation
- **Do NOT renumber** — leave gaps in numbering to preserve historical references

**What NOT to remove:**
- Plans marked `🟡 In Progress`, `⬜ Planned`, or `❌ Blocked` — regardless of age
- Plans that are `✅ Complete` but completed within the last 4 weeks
- Plans with active dependencies (other plans that reference this one)

**What IS removed (after 4 weeks):**
- Plans marked `✅ Complete` and older than 4 weeks
- Plans marked `⬜ Planned (superseded by ...)` and older than 4 weeks — rationale should be captured in a design document

---

## Master Progress Table

| # | Plan | Status | Created |
|---|------|--------|---------|
| 99 | [Unified Tie-Off Log and Direct Event Queueing](099-tie-offs-log-and-direct-queue/tie-offs-log-and-direct-queue-plan.md) | ⬜ Planned | 2026-10-23 |
| 98 | [Knot Lint — Static Rig Consistency Audit, with the Behaviour-in-Skill Rule](098-knot-lint/knot-lint-plan.md) | ⬜ Planned | 2026-10-22 |
| 97 | [Queue-Backed Pending-Event Visibility for Producers](097-queued-event-visibility/queued-event-visibility-plan.md) | 🟡 In Progress | 2026-10-18 |
| 96 | [Hard-Coded Tie-Off Contract — the Content-Only Override](096-hardcoded-tieoff-contract/hardcoded-tieoff-contract-plan.md) | ✅ Complete (2026-10-16) — released in v0.60.0 | 2026-10-16 |
| 95 | [Skill-Backed Tie-Off and Event-Content Instructions — `knot-template` Skill + Per-Rig Instruction Links](plan-skill-backed-tieoff-instructions.md) | ✅ Complete (2026-10-16) — released in v0.59.0 | 2026-10-16 |
| 94 | [Event-Enforcement Follow-Up Tie-Off Is Never Appended](plan-bugfix-event-enforcement-followup-tieoff.md) | ✅ Complete (2026-10-16) — released in v0.59.0 | 2026-10-16 |
| 93 | [Rig Graph Visualiser](093-rig-graph-visualiser/rig-graph-visualiser-plan.md) | ✅ Complete (2026-10-02) — v0.51.0–v0.58.0 (phases 0–10) | 2026-10-02 |
| 92 | [Static Engine Token for Rig-Scoped System Events, with Zero-Consumer Diagnostics](092-static-rig-event-token/static-rig-event-token-plan.md) | ✅ Complete (2026-09-21) — released in v0.50.0 | 2026-09-21 |
| 91 | [Per-Event Enforcement — Re-Ask When a Tie-Off Acknowledges Only Some Expected Events](091-per-event-enforcement/per-event-enforcement-plan.md) | ✅ Complete (2026-09-20) — released in v0.49.0 | 2026-09-20 |
| 71 | [Record the Pi Session ID in Tie-Off Sections](071-tie-off-session-id/tie-off-session-id-plan.md) | 📝 Draft | 2026-08-22 |
| 68 | [Rig Repository Separation](068-rig-repo-separation/rig-repo-separation-plan.md) | 📝 Draft | 2026-08-17 |
| 66 | [Relax the Knot Terminology Rule](066-terminology-rule-relaxation/terminology-rule-relaxation-plan.md) | 📝 Draft | 2026-07-29 |
| 63 | [Spurious Delete Suppression](063-spurious-delete-suppression/spurious-delete-suppression-plan.md) | 📝 Draft | 2026-07-16 |
| 31 | [Agent Profile Skills](agent-profile-skills.md) | ⬜ Planned | 2026-06-16 |
| 22 | [Notify Sender Leak Fix — Immediate Cascade Drain](notify-sender-leak-fix.md) | ⬜ Planned | 2026-06-11 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | 🟡 In Progress | 2026-06-08 |

---

_Overview sections for active and recently completed plans go here._

### 99. Unified Tie-Off Log and Direct Event Queueing

**Status:** ⬜ Planned (2026-10-23)
**Where it is:** Draft — awaiting activation
**Where it's going:** `tie-offs/<rig>/` holds only `state.json`, the service log, one accumulating `tie-offs.log` (sole-writer, structured section headers), and the `events/` queue; events queue directly from the tie-off parse as payload-embedded per-subscriber entries (dispatched-md files and the `event:`-URI strand path retire); commits carry trace trailers (`session`, `event`, `source`, `original_strand`) and the service-log grammar gains the same fields — the debug trace chain closes with no persistent payload duplication.
**Plan:** [099-tie-offs-log-and-direct-queue/tie-offs-log-and-direct-queue-plan.md](099-tie-offs-log-and-direct-queue/tie-offs-log-and-direct-queue-plan.md)

### 98. Knot Lint — Static Rig Consistency Audit, with the Behaviour-in-Skill Rule

**Status:** ⬜ Planned (2026-10-22)
**Where it is:** Draft — awaiting activation
**Where it's going:** the `knot-lint` sub-skill lints any rig for inline-behaviour knots and cross-document contract drift (advisory, never modifies), the behaviour-in-skill rule is normative in `knot-design` and the design guide, and the first dated report over the workspace's rigs lands in this project lifecycle.
**Plan:** [098-knot-lint/knot-lint-plan.md](098-knot-lint/knot-lint-plan.md)

### 96. Hard-Coded Tie-Off Contract — the Content-Only Override

**Status:** ✅ Complete (2026-10-16) — released in v0.60.0
**Created:** 2026-10-16
**Goal:** Move the tie-off **contract back to the hard-coded core**: the tie-off contract, the event format (`event:` / `occurred:` / `timestamp:`), and the rules (one block per listed event, always) are compiled into the binary and can no longer be overridden per rig — only the **tie-off content section** (the tie-off's header/narrative content + the event content guide: `description`, body, additional custom properties) is exposed, via a single `tie-off-content-instruction-file` link; the two 0.59.0 keys are retired (parsed, warned, not honoured); mechanics unchanged (per-session pull, local files only, default fallback, service writes nothing).

Full details in [096-hardcoded-tieoff-contract/hardcoded-tieoff-contract-plan.md](096-hardcoded-tieoff-contract/hardcoded-tieoff-contract-plan.md).

### 95. Skill-Backed Tie-Off and Event-Content Instructions — `knot-template` Skill + Per-Rig Instruction Links

**Status:** ✅ Complete (2026-10-16) — released in v0.59.0
**Created:** 2026-10-16
**Goal:** Make the tie-off contract and the event-content guide injected into producer knots' `# Subscriber Events` block changeable without a binary release: the instructions become a **hard-coded default compiled into the binary** (single source of truth: the template files of the new `knot-template` skill, via `include_str!`), with a per-rig **full-file override** named in `.workspace-agent-config.yaml` (`tie-off-instruction-file` / `event-content-instruction-file` — absolute, rig-relative, `~/...`, or `file://`; `http(s)` refused), pulled fresh on **every new session** (never cached); the `knot-template` skill ships the two templates and the procedure for adding a project override (`rig/knot-template/` + the config links). The service never writes instruction files into the rig. Plus parser hardening: `extract_emitted_event_ids` anchors on the subscriber bullet run instead of scanning the whole block.

Full details in [plan-skill-backed-tieoff-instructions.md](plan-skill-backed-tieoff-instructions.md).

### 94. Event-Enforcement Follow-Up Tie-Off Is Never Appended

**Status:** ✅ Complete (2026-10-16) — released in v0.59.0
**Created:** 2026-10-16
**Goal:** Fix the bug where the event-enforcement follow-up (the re-injected tie-off request after a malformed first tie-off) is dispatched but its response is **never appended to the tie-off file** — the second turn was invisible in `tie-offs/<rig>/<loom>/tie-off-<knot>.md`. The follow-up response is now appended to the **same tie-off file** as a second section (header `… (event-enforcement follow-up)`); an empty response is recorded as a short marker section; an errored follow-up appends nothing.

Full details in [plan-bugfix-event-enforcement-followup-tieoff.md](plan-bugfix-event-enforcement-followup-tieoff.md).

### 93. Rig Graph Visualiser

**Status:** ✅ Complete (2026-10-02) — phases 0–3 in v0.51.0, phase 4 in v0.52.0 (bugfix v0.52.1), phase 5 in v0.53.0, phase 6 in v0.54.0, phase 7 in v0.55.0
**Created:** 2026-10-02
**Completed:** 2026-10-02
**Completed:** 2026-10-02
**Goal:** Extract the rig's knot/strand graph (knots as nodes, `event:` subscriptions and filesystem `strand-dir`s as edges) via a stdlib-only Python script and render it to a self-contained, offline-viewable D3 HTML file at a user-specified `--out` path, packaged as the `knot-visualise` sub-skill.

Full details in [093-rig-graph-visualiser/rig-graph-visualiser-plan.md](093-rig-graph-visualiser/rig-graph-visualiser-plan.md).

### 71. Record the Pi Session ID in Tie-Off Sections

**Status:** 📝 Draft
**Created:** 2026-08-22
**Goal:** Record the pi session ID captured during agent execution as an optional `session:` metadata line in tie-off sections, so every tie-off section is traceable to the exact pi session that produced it.

Full details in [071-tie-off-session-id/tie-off-session-id-plan.md](071-tie-off-session-id/tie-off-session-id-plan.md).

### 63. Spurious Delete Suppression

**Status:** 📝 Draft
**Created:** 2026-07-16
**Goal:** Suppress spurious DELETE events caused by atomic file rewrites (truncate+write) using a configurable 5-second suppression window in the debounce engine.

Full details in [063-spurious-delete-suppression/spurious-delete-suppression-plan.md](063-spurious-delete-suppression/spurious-delete-suppression-plan.md).

### 31. Agent Profile Skills

**Status:** ⬜ Planned
**Created:** 2026-06-16
**Goal:** Add `skills` field to agent profiles so Knot passes `--no-skills` + `--skill <path>` to `pi`, making the agent's skill set explicit and keeping context concise.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [agent-profile-skills.md](agent-profile-skills.md).

### 22. Notify Sender Leak Fix — Immediate Cascade Drain

**Status:** ⬜ Planned
**Created:** 2026-06-11
**Goal:** Split `NotifyEventSource` senders from callback state so channels close immediately on drop, removing the 5-second timeout safety net.

Full details in [notify-sender-leak-fix.md](notify-sender-leak-fix.md).

### 20. Knot Modification Observability and Path Resolution Consistency

**Status:** 🟡 In Progress
**Created:** 2026-06-08
**Completed (Phase 0):** 2026-06-15
**Goal:** Make `KnotModified` filesystem changes observable via loom-log (`LoomEvent::KnotUpdated`), log parse failures to stderr, and ensure path resolution is consistent between initial load and file-watcher events.

**Result (Phase 0):** `NotifyEventSource` now receives correct `project_root` (parent of rig directory) so relative `strand_dir` paths resolve identically to `FileSystemLoomRepository::scan()`. Full rename `base_dir` → `rig_dir` across all 7 source files + 17 test files to eliminate ambiguity between "rig directory" and "project root". Remaining phases (KnotUpdated loom-log, parse failure logging, integration test) still pending.

Full details in [plan-knot-modify-observability.md](plan-knot-modify-observability.md).
